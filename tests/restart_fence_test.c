#include <libusb-1.0/libusb.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/wait.h>
#include <unistd.h>

int main(void) {
    pid_t child = fork();
    if (child < 0) {
        perror("fork");
        return 1;
    }
    if (child == 0) {
        unsigned char buffer[8] = {0};
        int transferred = -1;

        if (setenv("SMIUSB_GUARD_RESTART_ON_DISCONNECT", "1", 1) != 0) {
            _exit(90);
        }
        (void)libusb_bulk_transfer(NULL, 0x02, buffer, sizeof(buffer), &transferred, 10);
        _exit(91);
    }

    int status = 0;
    if (waitpid(child, &status, 0) != child) {
        perror("waitpid");
        return 1;
    }
    if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) {
        fprintf(stderr, "restart fence child status=%d, expected clean exit 0\n", status);
        return 1;
    }
    return 0;
}
