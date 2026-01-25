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
#include <unistd.h>

int main() {
    const char* path = NEXUS_ROOT "/elapsed_ms";
    setbuf(stdout, NULL);
    printf("Opening file at %s\n", path);
    int fd = open(path, O_RDONLY);
    char buf[64];
    if (fd < 0) {
        fprintf(stderr, "Error opening time file.");
        return 1;
    }
    for (size_t i = 0; i < 5; ++i) {
        if (read(fd, buf, sizeof(buf)) < 0) {
            fprintf(stderr, "Error reading time file.");
            return 2;
        }
        char* nptr = NULL;
        uint64_t ms_since_epoch = strtoull(buf, &nptr, 10);
        printf("Milliseconds Elapsed: %llu\n",
               (unsigned long long)ms_since_epoch);
        sleep(1);
    }
    exit(EXIT_SUCCESS);
}
