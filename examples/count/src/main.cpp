#include <stdio.h>
#include <stdlib.h>
#include <signal.h>
#include <sys/time.h>
#include <time.h>
#include <stdint.h>

int64_t COUNTER = 0;
volatile bool CONTINUE = true;

void alarm_handler(int signum) {
    if (signum == SIGALRM) {
        CONTINUE = false;
    }
}

int main() {
    signal(SIGALRM, alarm_handler);
    struct itimerval timer;
    timer.it_value.tv_sec = 1;
    timer.it_value.tv_usec = 0;
    timer.it_interval.tv_sec = 0;
    timer.it_interval.tv_usec = 0;
    setitimer(ITIMER_REAL, &timer, NULL);
    while (CONTINUE) {
        COUNTER += 1;
    }
    printf("%ld", COUNTER);
    return 0;
}
