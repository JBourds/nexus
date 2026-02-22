/**
 * elapsed/src/main.cpp
 *
 * Read from the "elapsed" file.
 */
#include <fcntl.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define NFILES 3

int FDS[NFILES];
const char* PATHS[NFILES] = {
    NEXUS_ROOT "/ctl.elapsed.s",
    NEXUS_ROOT "/ctl.elapsed.ms",
    NEXUS_ROOT "/ctl.elapsed.us",
};

void open_files();
void read_files();

int main() {
    setbuf(stdout, NULL);
    open_files();
    for (size_t i = 0; i < 3; ++i) {
        read_files();
        sleep(1);
    }
}

void open_files() {
    for (size_t i = 0; i < NFILES; ++i) {
        printf("Opening file at %s\n", PATHS[i]);
        int fd = open(PATHS[i], O_RDONLY);
        if (fd < 0) {
            fprintf(stderr, "Error opening elapsed file.");
            exit(EXIT_FAILURE);
        }
        FDS[i] = fd;
    }
}

void read_files() {
    char buf[64];
    for (size_t i = 0; i < NFILES; ++i) {
        ssize_t nread = read(FDS[i], buf, sizeof(buf));
        if (nread < 0) {
            fprintf(stderr, "Error reading elapsed file.");
            exit(EXIT_FAILURE);
        }
        buf[nread] = '\0';
        char* nptr = NULL;
        uint64_t ms_since_epoch = strtoull(buf, &nptr, 10);
        printf("%s Elapsed: %llu\n", PATHS[i],
               (unsigned long long)ms_since_epoch);
    }
}
