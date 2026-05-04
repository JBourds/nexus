/**
 * sleep/src/main.cpp
 *
 * Exercises both sleep control-file variants. Each iteration:
 *   1. Reads ctl.elapsed/us and prints `<elapsed_us>,start`.
 *   2. Sleeps relatively for SLEEP_REL_MS via ctl.sleep.relative/ms.
 *   3. Reads elapsed and prints `<elapsed_us>,after_relative`.
 *   4. Sleeps absolutely until elapsed_us + SLEEP_ABS_US via
 *      ctl.sleep.absolute/us.
 *   5. Reads elapsed and prints `<elapsed_us>,after_absolute`.
 *
 * Expected per-iteration deltas in elapsed_us:
 *   start -> after_relative   ~ SLEEP_REL_MS * 1000
 *   after_relative -> after_absolute ~ SLEEP_ABS_US
 */
#include <errno.h>
#include <fcntl.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define ITERATIONS 5
#define SLEEP_REL_MS 50
#define SLEEP_ABS_US 25000

static const char* ELAPSED_US_PATH = NEXUS_ROOT "/ctl.elapsed/us";
static const char* SLEEP_REL_MS_PATH = NEXUS_ROOT "/ctl.sleep.relative/ms";
static const char* SLEEP_ABS_US_PATH = NEXUS_ROOT "/ctl.sleep.absolute/us";

static int open_or_die(const char* path, int flags) {
    int fd = open(path, flags);
    if (fd < 0) {
        fprintf(stderr, "open %s failed: %s\n", path, strerror(errno));
        exit(1);
    }
    return fd;
}

static uint64_t read_elapsed_us(int fd) {
    char buf[32] = {0};
    ssize_t n = read(fd, buf, sizeof(buf) - 1);
    if (n < 0) {
        fprintf(stderr, "read elapsed failed: %s\n", strerror(errno));
        exit(1);
    }
    return strtoull(buf, NULL, 10);
}

/* Write a u64 as ASCII (no trailing newline) to a sleep control file.
 * The write blocks at the FUSE driver level until the kernel's sleep
 * dispatcher fires the wakeup at the requested simulated time. */
static void sleep_write(int fd, uint64_t value, const char* label) {
    char buf[24];
    int n = snprintf(buf, sizeof(buf), "%llu", (unsigned long long)value);
    if (n <= 0) {
        fprintf(stderr, "snprintf failed for %s\n", label);
        exit(1);
    }
    ssize_t written = write(fd, buf, (size_t)n);
    if (written < 0) {
        fprintf(stderr, "write %s failed: %s\n", label, strerror(errno));
        exit(1);
    }
}

int main() {
    setbuf(stdout, NULL);

    int elapsed_fd = open_or_die(ELAPSED_US_PATH, O_RDONLY);
    int sleep_rel_fd = open_or_die(SLEEP_REL_MS_PATH, O_WRONLY);
    int sleep_abs_fd = open_or_die(SLEEP_ABS_US_PATH, O_WRONLY);

    printf("elapsed_us,phase\n");

    for (int i = 0; i < ITERATIONS; ++i) {
        uint64_t t0 = read_elapsed_us(elapsed_fd);
        printf("%llu,start\n", (unsigned long long)t0);

        sleep_write(sleep_rel_fd, SLEEP_REL_MS, "sleep.relative/ms");

        uint64_t t1 = read_elapsed_us(elapsed_fd);
        printf("%llu,after_relative\n", (unsigned long long)t1);

        sleep_write(sleep_abs_fd, t1 + SLEEP_ABS_US, "sleep.absolute/us");

        uint64_t t2 = read_elapsed_us(elapsed_fd);
        printf("%llu,after_absolute\n", (unsigned long long)t2);
    }

    close(elapsed_fd);
    close(sleep_rel_fd);
    close(sleep_abs_fd);
    return 0;
}
