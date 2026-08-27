#include "smiusb/protocol.h"

#include <libusb-1.0/libusb.h>

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <csignal>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <deque>
#include <map>
#include <memory>
#include <mutex>
#include <string_view>
#include <thread>
#include <utility>

namespace {

std::atomic_bool stop_requested{false};

void request_stop(int signal_number) {
    static_cast<void>(signal_number);
    stop_requested.store(true, std::memory_order_relaxed);
}

struct UsbEvent {
    libusb_device *device;
    libusb_hotplug_event event;
};

struct DeviceSession {
    DeviceSession(libusb_device *owned_device, libusb_device_handle *owned_handle,
                  std::uint64_t session_generation)
        : device(owned_device), handle(owned_handle), generation(session_generation) {}

    DeviceSession(const DeviceSession &) = delete;
    DeviceSession &operator=(const DeviceSession &) = delete;

    ~DeviceSession() {
        /* This destructor runs only on SessionManager's worker thread. */
        if (handle != nullptr) {
            libusb_close(handle);
        }
        if (device != nullptr) {
            libusb_unref_device(device);
        }
    }

    libusb_device *device;
    libusb_device_handle *handle;
    std::uint64_t generation;
};

class SessionManager {
  public:
    SessionManager() : worker_(&SessionManager::worker_loop, this) {}

    SessionManager(const SessionManager &) = delete;
    SessionManager &operator=(const SessionManager &) = delete;

    ~SessionManager() { stop(); }

    void enqueue(libusb_device *device, libusb_hotplug_event event) {
        libusb_ref_device(device);
        {
            std::lock_guard lock(mutex_);
            if (stopping_) {
                libusb_unref_device(device);
                return;
            }
            queue_.push_back({device, event});
        }
        ready_.notify_one();
    }

    void stop() {
        {
            std::lock_guard lock(mutex_);
            if (stopping_) {
                return;
            }
            stopping_ = true;
        }
        ready_.notify_one();
        if (worker_.joinable()) {
            worker_.join();
        }
    }

  private:
    void worker_loop() {
        for (;;) {
            UsbEvent event{};
            {
                std::unique_lock lock(mutex_);
                ready_.wait(lock, [this] { return stopping_ || !queue_.empty(); });
                if (queue_.empty() && stopping_) {
                    break;
                }
                event = queue_.front();
                queue_.pop_front();
            }

            if (event.event == LIBUSB_HOTPLUG_EVENT_DEVICE_ARRIVED) {
                handle_arrival(event.device);
            } else {
                handle_departure(event.device);
            }
            libusb_unref_device(event.device);
        }

        for (const UsbEvent &event : queue_) {
            libusb_unref_device(event.device);
        }
        queue_.clear();
        sessions_.clear();
    }

    static bool is_target(libusb_device *device) {
        libusb_device_descriptor descriptor{};
        return libusb_get_device_descriptor(device, &descriptor) == LIBUSB_SUCCESS &&
               descriptor.idVendor == SMIUSB_VENDOR_ID &&
               descriptor.idProduct == SMIUSB_SM768_PRODUCT_ID;
    }

    void handle_arrival(libusb_device *device) {
        if (!is_target(device) || sessions_.contains(device)) {
            return;
        }

        libusb_device_handle *handle = nullptr;
        int result = libusb_open(device, &handle);
        if (result != LIBUSB_SUCCESS) {
            std::fprintf(stderr, "smiusbd: open failed for bus=%u address=%u: %s\n",
                         libusb_get_bus_number(device), libusb_get_device_address(device),
                         libusb_error_name(result));
            return;
        }

        /* Transfer the session's own device reference into DeviceSession. */
        libusb_ref_device(device);
        auto session = std::make_unique<DeviceSession>(device, handle, ++generation_);
        std::fprintf(stderr,
                     "smiusbd: generation=%lu arrived bus=%u address=%u "
                     "(observe-only; no interface claimed)\n",
                     static_cast<unsigned long>(session->generation), libusb_get_bus_number(device),
                     libusb_get_device_address(device));
        sessions_.emplace(device, std::move(session));
    }

    void handle_departure(libusb_device *device) {
        auto iterator = sessions_.find(device);
        if (iterator == sessions_.end()) {
            return;
        }

        std::unique_ptr<DeviceSession> session = std::move(iterator->second);
        sessions_.erase(iterator);
        std::fprintf(stderr, "smiusbd: generation=%lu departed; closing on worker thread\n",
                     static_cast<unsigned long>(session->generation));
        session.reset();
    }

    std::mutex mutex_;
    std::condition_variable ready_;
    std::deque<UsbEvent> queue_;
    std::map<libusb_device *, std::unique_ptr<DeviceSession>> sessions_;
    bool stopping_{false};
    std::thread worker_;
    std::uint64_t generation_{0};
};

int hotplug_callback(libusb_context *context, libusb_device *device, libusb_hotplug_event event,
                     void *user_data) {
    static_cast<void>(context);
    auto *manager = static_cast<SessionManager *>(user_data);
    manager->enqueue(device, event);
    return 0;
}

struct Options {
    unsigned int duration_seconds{0};
};

bool parse_options(int argc, char **argv, Options &options) {
    if (argc < 2 || std::string_view(argv[1]) != "--observe") {
        return false;
    }
    if (argc == 2) {
        return true;
    }
    if (argc == 4 && std::string_view(argv[2]) == "--duration") {
        char *end = nullptr;
        unsigned long duration = std::strtoul(argv[3], &end, 10);
        if (end != argv[3] && *end == '\0' && duration > 0 && duration <= UINT32_MAX) {
            options.duration_seconds = static_cast<unsigned int>(duration);
            return true;
        }
    }
    return false;
}

} // namespace

int main(int argc, char **argv) {
    Options options;
    if (!parse_options(argc, argv, options)) {
        std::fprintf(stderr,
                     "usage: %s --observe [--duration SECONDS]\n"
                     "observe mode opens the SM768 but never claims an interface "
                     "or sends data\n",
                     argv[0]);
        return 2;
    }

    libusb_context *context = nullptr;
    int result = libusb_init(&context);
    if (result != LIBUSB_SUCCESS) {
        std::fprintf(stderr, "smiusbd: libusb_init: %s\n", libusb_error_name(result));
        return 1;
    }

    std::signal(SIGINT, request_stop);
    std::signal(SIGTERM, request_stop);

    SessionManager sessions;
    libusb_hotplug_callback_handle callback_handle{};
    result = libusb_hotplug_register_callback(
        context, LIBUSB_HOTPLUG_EVENT_DEVICE_ARRIVED | LIBUSB_HOTPLUG_EVENT_DEVICE_LEFT,
        LIBUSB_HOTPLUG_ENUMERATE, SMIUSB_VENDOR_ID, SMIUSB_SM768_PRODUCT_ID,
        LIBUSB_HOTPLUG_MATCH_ANY, hotplug_callback, &sessions, &callback_handle);
    if (result != LIBUSB_SUCCESS) {
        std::fprintf(stderr, "smiusbd: hotplug registration: %s\n", libusb_error_name(result));
        sessions.stop();
        libusb_exit(context);
        return 1;
    }

    const auto deadline =
        options.duration_seconds == 0
            ? std::chrono::steady_clock::time_point::max()
            : std::chrono::steady_clock::now() + std::chrono::seconds(options.duration_seconds);
    while (!stop_requested.load(std::memory_order_relaxed) &&
           std::chrono::steady_clock::now() < deadline) {
        timeval timeout{.tv_sec = 0, .tv_usec = 250000};
        result = libusb_handle_events_timeout_completed(context, &timeout, nullptr);
        if (result != LIBUSB_SUCCESS && result != LIBUSB_ERROR_INTERRUPTED) {
            std::fprintf(stderr, "smiusbd: event loop: %s\n", libusb_error_name(result));
            break;
        }
    }

    libusb_hotplug_deregister_callback(context, callback_handle);
    sessions.stop();
    libusb_exit(context);
    return result == LIBUSB_SUCCESS || result == LIBUSB_ERROR_INTERRUPTED ? 0 : 1;
}
