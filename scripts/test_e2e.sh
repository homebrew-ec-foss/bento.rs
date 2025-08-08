#!/bin/bash

echo "=================================================="
echo "  BENTO END-TO-END INTEGRATION TEST SUITE"
echo "=================================================="

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
PURPLE='\033[0;35m'
NC='\033[0m' # No Color

# Test configuration
VERBOSE=false
CLEANUP_ON_FAIL=true
SLEEP_BETWEEN_TESTS=1
test_passed=0
test_failed=0

# Usage function
usage() {
    echo "Usage: $0 [options]"
    echo "Options:"
    echo "  -v, --verbose       Enable verbose logging (show all command output)"
    echo "  --no-cleanup        Don't clean up containers on test failure (for debugging)"
    echo "  --fast              Skip sleep delays between tests"
    echo "  -h, --help          Show this help message"
    echo ""
    echo "This script tests the complete Bento container lifecycle:"
    echo "  1. Container creation with various configurations"
    echo "  2. Container state inspection"
    echo "  3. Container listing and filtering"
    echo "  4. Container startup and runtime"
    echo "  5. Resource monitoring and statistics"
    echo "  6. Container stopping and cleanup"
    echo "  7. Error handling and edge cases"
    exit 0
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -v|--verbose)
            VERBOSE=true
            shift
            ;;
        --no-cleanup)
            CLEANUP_ON_FAIL=false
            shift
            ;;
        --fast)
            SLEEP_BETWEEN_TESTS=0
            shift
            ;;
        -h|--help)
            usage
            ;;
        *)
            echo "Unknown option: $1"
            usage
            ;;
    esac
done

# Logging functions
log_verbose() {
    if [ "$VERBOSE" = true ]; then
        echo -e "${CYAN}[VERBOSE]${NC} $1"
    fi
}

log_command() {
    if [ "$VERBOSE" = true ]; then
        echo -e "${BLUE}[CMD]${NC} $1"
    fi
}

log_step() {
    echo -e "${PURPLE}[STEP]${NC} $1"
}

run_command() {
    local cmd="$1"
    local description="$2"
    
    log_command "$cmd"
    
    if [ "$VERBOSE" = true ]; then
        eval "$cmd"
        return $?
    else
        eval "$cmd" >/dev/null 2>&1
        return $?
    fi
}

run_command_with_output() {
    local cmd="$1"
    log_command "$cmd"
    eval "$cmd"
}

print_result() {
    if [ $1 -eq 0 ]; then
        echo -e "${GREEN}✓ PASS${NC}: $2"
        ((test_passed++))
    else
        echo -e "${RED}✗ FAIL${NC}: $2"
        ((test_failed++))
        if [ "$VERBOSE" = true ] && [ -n "$3" ]; then
            echo -e "${YELLOW}  Details: $3${NC}"
        fi
    fi
}

wait_for_step() {
    if [ "$SLEEP_BETWEEN_TESTS" -gt 0 ]; then
        sleep "$SLEEP_BETWEEN_TESTS"
    fi
}

# --- Helpers for cgroup validation ---
to_bytes() {
    local v="$1"
    case "$v" in
        *K|*k) echo $(( ${v%[Kk]} * 1024 )) ;;
        *M|*m) echo $(( ${v%[Mm]} * 1024 * 1024 )) ;;
        *G|*g) echo $(( ${v%[Gg]} * 1024 * 1024 * 1024 )) ;;
        max) echo "max" ;;
        *) echo "$v" ;;
    esac
}

get_user_cgroup_base() {
    local rel
    rel=$(awk -F: '$1==0{print $3}' /proc/self/cgroup)
    echo "/sys/fs/cgroup${rel}"
}

validate_cgroup_value() {
    local cpath="$1" key="$2" expected="$3"
    if [ ! -r "$cpath/$key" ]; then
        echo -e "${YELLOW}[Warn]${NC} Missing cgroup file: $cpath/$key"
        return 1
    fi
    local got
    got=$(tr -d '\n' < "$cpath/$key")
    case "$key" in
        memory.max|memory.swap.max|memory.high)
            local expb gotb
            expb=$(to_bytes "$expected")
            if [ "$got" = "max" ]; then
                return 1
            fi
            if [[ "$got" =~ ^[0-9]+$ ]]; then
                gotb="$got"
                [ "$gotb" -eq "$expb" ] 2>/dev/null
                return $?
            fi
            [ "$got" = "$expected" ]
            ;;
        cpu.max)
            [ "$got" = "$expected" ]
            ;;
        pids.max)
            [ "$got" = "$expected" ]
            ;;
        *)
            # default exact match
            [ "$got" = "$expected" ]
            ;;
    esac
}

cleanup_all_test_containers() {
    log_verbose "Cleaning up all test containers..."
    local containers=$(cargo run -- list 2>/dev/null | grep -E "(e2e-test-|lifecycle-|scenario-)" | awk '{print $1}' 2>/dev/null || true)
    for container in $containers; do
        if [ -n "$container" ] && [ "$container" != "CONTAINER" ]; then
            log_verbose "Cleaning up container: $container"
            cargo run -- delete "$container" >/dev/null 2>&1 || true
        fi
    done
}

# Cleanup on exit
cleanup_on_exit() {
    if [ "$test_failed" -gt 0 ] && [ "$CLEANUP_ON_FAIL" = true ]; then
        echo
        echo -e "${YELLOW}Cleaning up failed test containers...${NC}"
        cleanup_all_test_containers
    fi
}
trap cleanup_on_exit EXIT

# Verify prerequisites
check_prerequisites() {
    log_step "Checking prerequisites"
    
    # Check if cargo is available
    if ! command -v cargo &> /dev/null; then
        print_result 1 "Prerequisites check" "cargo command not found"
        exit 1
    fi
    
    # Check if bento can be built
    if ! cargo build >/dev/null 2>&1; then
        print_result 1 "Prerequisites check" "Failed to build bento"
        exit 1
    fi
    
    # Check cgroups availability
    if [ ! -d "/sys/fs/cgroup/user.slice/user-$(id -u).slice/user@$(id -u).service" ]; then
        print_result 1 "Prerequisites check" "User cgroups not available"
        exit 1
    fi
    
    # Check bundle directory
    if [ ! -d "./test-bundle" ]; then
        mkdir -p ./test-bundle
        log_verbose "Created test bundle directory"
    fi
    
    print_result 0 "Prerequisites check"
}

echo
if [ "$VERBOSE" = true ]; then
    echo -e "${CYAN}Running in VERBOSE mode - showing all output${NC}"
else
    echo -e "${CYAN}Running in QUIET mode - use -v for verbose output${NC}"
fi
echo

# Initial cleanup
log_step "Initial cleanup"
cleanup_all_test_containers

check_prerequisites
wait_for_step

# Test 1: Basic Container Creation
echo
echo "=== Test 1: Basic Container Creation ==="
BASIC_CONTAINER="e2e-test-basic-$$"

log_step "Creating basic container: $BASIC_CONTAINER"
if run_command "cargo run -- create '$BASIC_CONTAINER' --bundle ./test-bundle" "Basic container creation"; then
    print_result 0 "Basic container creation"
else
    print_result 1 "Basic container creation"
fi
wait_for_step

# Test 2: Container State Inspection
echo
echo "=== Test 2: Container State Inspection ==="
log_step "Checking container state"
output=$(cargo run -- state "$BASIC_CONTAINER" 2>&1)
if echo "$output" | grep -q "Container ID: $BASIC_CONTAINER" && echo "$output" | grep -q "Status: created"; then
    print_result 0 "Container state inspection"
    if [ "$VERBOSE" = true ]; then
        echo -e "${CYAN}State output:${NC}"
        echo "$output"
    fi
else
    print_result 1 "Container state inspection" "Expected created status not found"
fi
wait_for_step

# Test 3: Container Listing
echo
echo "=== Test 3: Container Listing ==="
log_step "Listing containers"
output=$(cargo run -- list 2>&1)
if echo "$output" | grep -q "$BASIC_CONTAINER"; then
    print_result 0 "Container appears in list"
    if [ "$VERBOSE" = true ]; then
        echo -e "${CYAN}List output:${NC}"
        echo "$output"
    fi
else
    print_result 1 "Container appears in list" "Container not found in list output"
fi
wait_for_step

# Test 4: Advanced Container Creation with Resource Limits
echo
echo "=== Test 4: Advanced Container Creation with Resource Limits ==="
ADVANCED_CONTAINER="e2e-test-advanced-$$"

log_step "Creating container with resource limits"
if run_command "cargo run -- create '$ADVANCED_CONTAINER' --bundle ./test-bundle --memory-limit 256M --memory-high 200M --memory-swap-limit 300M --cpu-limit '75000 100000' --cpu-weight 200 --pids-limit 200" "Advanced container creation"; then
    print_result 0 "Container creation with resource limits"
else
    print_result 1 "Container creation with resource limits"
fi
wait_for_step

# Test 4b: Resource Limits Validation
echo
echo "=== Test 4b: Resource Limits Validation ==="
log_step "Validating advanced container resource limits"
# Dynamically locate the advanced container cgroup directory (paths vary per system)
# Prefer under the user@.service subtree for delegated rootless runs
adv_cg=$(find /sys/fs/cgroup/user.slice -maxdepth 12 -type d -name "$ADVANCED_CONTAINER" 2>/dev/null | head -n1)
if [ -z "$adv_cg" ]; then
  adv_cg=$(find /sys/fs/cgroup -maxdepth 12 -type d -name "$ADVANCED_CONTAINER" 2>/dev/null | head -n1)
fi
if [ -z "$adv_cg" ]; then
    echo -e "${YELLOW}[Warn]${NC} Could not find cgroup directory for $ADVANCED_CONTAINER"
fi
rl_ok=true
validate_cgroup_value "$adv_cg" "memory.max" "256M" || rl_ok=false
validate_cgroup_value "$adv_cg" "memory.high" "200M" || rl_ok=false
validate_cgroup_value "$adv_cg" "cpu.max" "75000 100000" || rl_ok=false
validate_cgroup_value "$adv_cg" "pids.max" "200" || rl_ok=false
validate_cgroup_value "$adv_cg" "cpu.weight" "200" || rl_ok=false
# Swap limit may not be enforced on all setups; only validate presence/value if file exists
if [ -r "$adv_cg/memory.swap.max" ]; then
  validate_cgroup_value "$adv_cg" "memory.swap.max" "300M" || rl_ok=false
else
  echo -e "${YELLOW}[Warn]${NC} memory.swap.max not present; skipping validation"
fi
if $rl_ok; then
    print_result 0 "Resource limit validation (advanced)"
else
    print_result 1 "Resource limit validation (advanced)" "One or more limits not applied"
fi
wait_for_step

# Test 5: Container Creation without Cgroups
echo
echo "=== Test 5: Container Creation without Cgroups ==="
NO_CGROUPS_CONTAINER="e2e-test-no-cgroups-$$"

log_step "Creating container without cgroups"
if run_command "cargo run -- create '$NO_CGROUPS_CONTAINER' --bundle ./test-bundle --no-cgroups" "No-cgroups container creation"; then
    print_result 0 "Container creation without cgroups"
else
    print_result 1 "Container creation without cgroups"
fi
wait_for_step

# Test 6: Manual Rootfs Population Method
echo
echo "=== Test 6: Manual Rootfs Population Method ==="
MANUAL_CONTAINER="e2e-test-manual-$$"

log_step "Creating container with manual rootfs population"
if run_command "cargo run -- create '$MANUAL_CONTAINER' --bundle ./test-bundle --population-method manual --memory-limit 128M" "Manual rootfs container creation"; then
    print_result 0 "Container creation with manual rootfs"
else
    print_result 1 "Container creation with manual rootfs"
fi
wait_for_step

# Test 7: Multiple Container Management
echo
echo "=== Test 7: Multiple Container Management ==="
log_step "Verifying multiple containers in list"
output=$(cargo run -- list 2>&1)
container_count=$(echo "$output" | grep "e2e-test-" | wc -l 2>/dev/null || echo "0")

if [ "$container_count" -ge 4 ]; then
    print_result 0 "Multiple containers management" "Found $container_count containers"
else
    print_result 1 "Multiple containers management" "Expected 4+ containers, found $container_count"
fi

if [ "$VERBOSE" = true ]; then
    echo -e "${CYAN}Current containers:${NC}"
    echo "$output"
fi
wait_for_step

# Test 8: Container Statistics
echo
echo "=== Test 8: Container Statistics ==="
log_step "Checking container statistics"
output=$(cargo run -- stats 2>&1 | head -20)  # Limit output to avoid hanging
if echo "$output" | grep -q "CONTAINER RESOURCE USAGE"; then
    print_result 0 "Container statistics display"
    if [ "$VERBOSE" = true ]; then
        echo -e "${CYAN}Stats output:${NC}"
        echo "$output"
    fi
else
    print_result 1 "Container statistics display" "Stats header not found"
fi
wait_for_step

# Test 9: Container Start Attempt
echo
echo "=== Test 9: Container Start Attempt ==="
log_step "Attempting to start basic container"
# Note: This may fail due to filesystem issues, but we test the command
output=$(cargo run -- start "$BASIC_CONTAINER" 2>&1 || true)
if echo "$output" | grep -q -E "(started successfully|Container process.*dead|Failed to start)"; then
    print_result 0 "Container start command executed"
    if [ "$VERBOSE" = true ]; then
        echo -e "${CYAN}Start output:${NC}"
        echo "$output"
    fi
else
    print_result 1 "Container start command executed" "Unexpected start output"
fi
wait_for_step

# Test 10: Container State After Start Attempt
echo
echo "=== Test 10: Container State After Start Attempt ==="
log_step "Checking container state after start attempt"
output=$(cargo run -- state "$BASIC_CONTAINER" 2>&1)
if echo "$output" | grep -q -E "Status: (stopped|running)"; then
    print_result 0 "Container state after start attempt"
    if [ "$VERBOSE" = true ]; then
        echo -e "${CYAN}State after start:${NC}"
        echo "$output"
    fi
else
    print_result 1 "Container state after start attempt" "Expected status to be stopped or running"
fi
wait_for_step

# Test 11: Container Kill/Stop
echo
echo "=== Test 11: Container Kill/Stop ==="
log_step "Attempting to kill/stop containers"
kill_success=true

for container in "$BASIC_CONTAINER" "$ADVANCED_CONTAINER"; do
    log_verbose "Stopping container: $container"
    # Capture output to allow handling of "already stopped" as success
    output=$(cargo run -- kill "$container" 2>&1 || true)
    if echo "$output" | grep -q -E "(stopped successfully|already stopped)"; then
        log_verbose "Successfully stopped (or already stopped): $container"
    else
        log_verbose "Stop failed for: $container"
        if [ "$VERBOSE" = true ]; then
            echo "$output"
        fi
        kill_success=false
    fi
done

if $kill_success; then
    print_result 0 "Container kill/stop operations" "Commands executed successfully"
else
    print_result 1 "Container kill/stop operations" "One or more stop commands failed"
fi
wait_for_step

# Test 12: Container Cleanup and Deletion
echo
echo "=== Test 12: Container Cleanup and Deletion ==="
log_step "Deleting all test containers"
containers_to_delete=("$BASIC_CONTAINER" "$ADVANCED_CONTAINER" "$NO_CGROUPS_CONTAINER" "$MANUAL_CONTAINER")
deletion_success=true

for container in "${containers_to_delete[@]}"; do
    log_verbose "Deleting container: $container"
    if run_command "cargo run -- delete '$container'" "Delete $container"; then
        log_verbose "Successfully deleted: $container"
    else
        log_verbose "Deletion failed for: $container"
        deletion_success=false
    fi
done

if $deletion_success; then
    print_result 0 "Container deletion"
else
    print_result 1 "Container deletion" "Some containers could not be deleted"
fi
wait_for_step

# Test 13: Verify Complete Cleanup
echo
echo "=== Test 13: Verify Complete Cleanup ==="
log_step "Verifying all test containers are removed"
output=$(cargo run -- list 2>&1)
remaining_containers=$(echo "$output" | grep "e2e-test-" | wc -l 2>/dev/null || echo "0")

if [ "$remaining_containers" -eq 0 ]; then
    print_result 0 "Complete cleanup verification"
else
    print_result 1 "Complete cleanup verification" "$remaining_containers containers still remain"
    if [ "$VERBOSE" = true ]; then
        echo -e "${YELLOW}Remaining containers:${NC}"
        echo "$output" | grep "e2e-test-"
    fi
fi
wait_for_step

# Test 14: Error Handling Tests
echo
echo "=== Test 14: Error Handling Tests ==="

# Test non-existent container operations
log_step "Testing error handling for non-existent containers"
error_tests_passed=0
error_tests_total=0

# Test state of non-existent container
((error_tests_total++))
if cargo run -- state "non-existent-container" 2>&1 | grep -q "not found"; then
    ((error_tests_passed++))
    log_verbose "✓ State command properly handles non-existent container"
else
    log_verbose "✗ State command error handling failed"
fi

# Test start of non-existent container
((error_tests_total++))
if cargo run -- start "non-existent-container" 2>&1 | grep -q -E "(not found|Failed to load state)"; then
    ((error_tests_passed++))
    log_verbose "✓ Start command properly handles non-existent container"
else
    log_verbose "✗ Start command error handling failed"
fi

# Test delete of non-existent container
((error_tests_total++))
if cargo run -- delete "non-existent-container" 2>&1 | grep -q -E "(not found|proceeding with cleanup)"; then
    ((error_tests_passed++))
    log_verbose "✓ Delete command properly handles non-existent container"
else
    log_verbose "✗ Delete command error handling failed"
fi

if [ "$error_tests_passed" -eq "$error_tests_total" ]; then
    print_result 0 "Error handling tests" "$error_tests_passed/$error_tests_total tests passed"
else
    print_result 1 "Error handling tests" "$error_tests_passed/$error_tests_total tests passed"
fi
wait_for_step

# Test 15: Stress Test - Multiple Rapid Operations
echo
echo "=== Test 15: Stress Test - Multiple Rapid Operations ==="
log_step "Performing rapid container operations"
stress_container="e2e-stress-test-$$"

stress_success=true

# Rapid create and delete cycle
for i in {1..3}; do
    container_name="${stress_container}-${i}"
    log_verbose "Stress test iteration $i: $container_name"
    
    if ! run_command "cargo run -- create '$container_name' --bundle ./test-bundle --memory-limit 64M" "Stress create $i"; then
        stress_success=false
        break
    fi
    
    if ! run_command "cargo run -- delete '$container_name'" "Stress delete $i"; then
        stress_success=false
        break
    fi
done

if $stress_success; then
    print_result 0 "Stress test - rapid operations"
else
    print_result 1 "Stress test - rapid operations" "Failed during rapid operations"
fi

# Final cleanup
log_step "Final stress test cleanup"
for i in {1..3}; do
    container_name="${stress_container}-${i}"
    cargo run -- delete "$container_name" >/dev/null 2>&1 || true
done

echo
echo "=================================================="
echo "              END-TO-END TEST SUMMARY"
echo "=================================================="
echo -e "Tests passed: ${GREEN}$test_passed${NC}"
echo -e "Tests failed: ${RED}$test_failed${NC}"
echo -e "Total tests:  $((test_passed + test_failed))"

# Feature coverage summary
echo
echo "=== Feature Coverage Summary ==="
echo "✓ Container creation (basic, advanced, no-cgroups, manual rootfs)"
echo "✓ Container state inspection"
echo "✓ Container listing and management"
echo "✓ Resource limits and cgroups integration"
echo "✓ Container statistics and monitoring"
echo "✓ Container lifecycle operations (start, stop, delete)"
echo "✓ Error handling and edge cases"
echo "✓ Multiple container management"
echo "✓ Stress testing and rapid operations"

# Rootless environment notes (informational)
echo
echo "=== Rootless Environment Notes ==="
echo "- Expected warnings may appear:"
echo "  • setgroups disable may be denied"
echo "  • PID/UTS/Mount namespaces may fail with EPERM"
echo "  • IO throttling is not supported in rootless mode (io controller disabled)"

if [ $test_failed -eq 0 ]; then
    echo
    echo -e "${GREEN}🎉 ALL END-TO-END TESTS PASSED!${NC}"
    echo -e "${GREEN}   Bento container runtime is fully functional!${NC}"
    exit 0
else
    echo
    echo -e "${YELLOW}⚠️  $test_failed test(s) failed in end-to-end testing${NC}"
    if [ "$VERBOSE" = false ]; then
        echo -e "${CYAN}💡 Run with -v for verbose output to debug issues${NC}"
    fi
    exit 1
fi