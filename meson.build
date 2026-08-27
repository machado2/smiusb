project(
  'smiusb-open',
  ['c', 'cpp'],
  version: '0.1.0',
  license: 'MIT',
  default_options: [
    'c_std=c11',
    'cpp_std=c++20',
    'warning_level=3',
    'werror=true',
  ],
)

add_project_arguments('-D_GNU_SOURCE', language: 'c')

cc = meson.get_compiler('c')
dl_dep = cc.find_library('dl')
threads_dep = dependency('threads')
libusb_dep = dependency('libusb-1.0')
protocol_include = include_directories('include')

guard = shared_module(
  'smiusb_guard',
  'src/guard/smiusb_guard.c',
  dependencies: [dl_dep, threads_dep],
  name_prefix: 'lib',
  install: true,
  install_dir: join_paths(get_option('prefix'), 'lib', 'smiusb'),
)

probe = executable(
  'smiusb-probe',
  'src/probe/smiusb_probe.c',
  dependencies: libusb_dep,
  install: true,
)

smiusbd = executable(
  'smiusbd',
  'src/daemon/smiusbd.cpp',
  include_directories: protocol_include,
  dependencies: [libusb_dep, threads_dep],
  install: true,
)

null_transfer_test = executable(
  'null-transfer-test',
  'tests/null_transfer_test.c',
  dependencies: libusb_dep,
)

restart_fence_test = executable(
  'restart-fence-test',
  'tests/restart_fence_test.c',
  dependencies: libusb_dep,
)

protocol_layout_test = executable(
  'protocol-layout-test',
  'tests/protocol_layout_test.c',
  include_directories: protocol_include,
)

guard_test_env = environment()
guard_test_env.set('LD_PRELOAD', guard.full_path())
test(
  'guard rejects null libusb handles',
  null_transfer_test,
  env: guard_test_env,
)
test(
  'guard converts disconnect into controlled restart',
  restart_fence_test,
  env: guard_test_env,
)
test('protocol structure layout', protocol_layout_test)
