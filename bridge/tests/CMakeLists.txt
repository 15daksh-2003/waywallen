add_executable(waywallen-bridge-pool-state-tests pool_state_tests.c)
target_include_directories(waywallen-bridge-pool-state-tests PRIVATE ../src)
target_link_libraries(waywallen-bridge-pool-state-tests PRIVATE waywallen::bridge Threads::Threads)
add_test(NAME waywallen-bridge-pool-state-tests COMMAND waywallen-bridge-pool-state-tests)
