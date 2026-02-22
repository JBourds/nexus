/**
 * time/src/main.cpp
 *
 * Read from the "time" file.
 */
#include <fcntl.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#include <cassert>
#include <cstdlib>

#define NFILES 3
#define SECONDS 0
#define MILLIS 1
#define MICROS 2

#define EXPECTED_START_S 1767268800

#define JAN_1_2026_S 1767225600
#define JAN_1_2026_MS 1767225600000
#define JAN_1_2026_US 1767225600000000

int FDS[NFILES];
const char* PATHS[NFILES] = {
    NEXUS_ROOT "/ctl.time.s",
    NEXUS_ROOT "/ctl.time.ms",
    NEXUS_ROOT "/ctl.time.us",
};

void open_files();
void read_files();
uint64_t read_file(int fd);
void write_time(size_t index, uint64_t val);

int main() {
    setbuf(stdout, NULL);
    open_files();
    assert(read_file(FDS[SECONDS]) == EXPECTED_START_S);
    read_files();

    write_time(SECONDS, JAN_1_2026_S);
    sleep(1);
    read_files();

    write_time(MILLIS, JAN_1_2026_MS);
    sleep(1);
    read_files();

    write_time(MICROS, JAN_1_2026_US);
    sleep(1);
    read_files();
}

void open_files() {
    for (size_t i = 0; i < NFILES; ++i) {
        printf("Opening file at %s\n", PATHS[i]);
        int fd = open(PATHS[i], O_RDWR);
        if (fd < 0) {
            fprintf(stderr, "Error opening time file.");
            exit(EXIT_FAILURE);
        }
        FDS[i] = fd;
    }
}

uint64_t read_file(int fd) {
    char buf[64];
    ssize_t nread = read(fd, buf, sizeof(buf));
    if (nread < 0) {
        fprintf(stderr, "Error reading time file.");
        exit(EXIT_FAILURE);
    }
    buf[nread] = '\0';
    char* nptr = NULL;
    uint64_t val = strtoull(buf, &nptr, 10);
    return val;
}

void read_files() {
    for (size_t i = 0; i < NFILES; ++i) {
        uint64_t val = read_file(FDS[i]);
        printf("%s Epoch: %llu\n", PATHS[i], (unsigned long long)val);
    }
}
void write_time(size_t index, uint64_t val) {
    char buf[64];
    int n = snprintf(buf, sizeof(buf), "%llu", (unsigned long long)val);
    assert(n > 0);
    size_t bytes_required = (size_t)n;
    if (sizeof(buf) < bytes_required) {
        fprintf(stderr, "Buffer was too small to fit %llu bytes",
                (unsigned long long)n);
        exit(EXIT_FAILURE);
    }
    ssize_t rc = write(FDS[index], buf, bytes_required);
    assert(rc > 0);
    if ((size_t)rc != bytes_required) {
        fprintf(
            stderr,
            "Didn't write correct number of bytes (got %lld expected %lld)\n",
            (long long)rc, (long long)n);
        exit(EXIT_FAILURE);
    }
}
