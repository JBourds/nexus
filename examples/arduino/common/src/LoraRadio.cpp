#include "LoraRadio.h"

#ifdef SIMULATE
#include <fcntl.h>
#include <sys/time.h>
#include <unistd.h>

#include <iostream>
#endif

#define MS_IN_SECOND 1000

namespace lora {

static const uint8_t LORA_CS = 10;
static const uint8_t LORA_INT = 2;
static const uint8_t LORA_RST = 9;
static const float LORA_FREQUENCY = 915.0;

static bool INITIALIZED = false;
static int16_t LAST_RSSI = 1;

#ifdef SIMULATE
#ifndef NEXUS_LORA
#error "\"NEXUS_LORA\" must be defined by simulation to locate file path"
#endif
static int RF95;
#else
static RH_RF95 RF95(LORA_CS, LORA_INT);
#endif

bool is_active() { return INITIALIZED; }

#ifndef SIMULATE
RH_RF95* get() { return INITIALIZED ? &RF95 : nullptr; }
#endif

int16_t last_rssi() { return LAST_RSSI; }

RC init() {
#ifdef SIMULATE
    RF95 = open(NEXUS_LORA, O_RDWR);
    if (RF95 == -1) {
        return RC::InitFailed;
    }
#else
    pinMode(3, OUTPUT);
    pinMode(5, OUTPUT);
    digitalWrite(3, HIGH);
    digitalWrite(5, HIGH);

    pinMode(LORA_RST, OUTPUT);
    digitalWrite(LORA_RST, HIGH);

    digitalWrite(LORA_RST, LOW);
    delay(10);
    digitalWrite(LORA_RST, HIGH);
    delay(10);

    if (!RF95.init()) {
        return RC::InitFailed;
    }
    if (!RF95.setFrequency(LORA_FREQUENCY)) {
        return RC::SetFrequencyFailed;
    }
#endif

    INITIALIZED = true;

    return RC::Okay;
}

RC deinit() {
    if (!INITIALIZED) {
        return RC::NotInit;
    }

#ifdef SIMULATE
    if (close(RF95) == -1) {
        return RC::DeinitFailed;
    }
#else
    // Forfeit SPI line
    pinMode(LORA_CS, INPUT);
#endif
    INITIALIZED = false;

    return RC::Okay;
}

RC send(const uint8_t buf[], size_t sz) {
    RC rc = RC::Okay;
    if (!is_active()) {
        rc = init();
    }

#ifdef SIMULATE
    ssize_t nwritten = write(RF95, buf, sz);
    if (nwritten < 0 || (size_t)nwritten != sz) {
        rc = RC::SendFailed;
    }
#else
    RH_RF95* rf95 = lora::get();

    if (rc == RC::Okay && !rf95->send(buf, sz)) {
        rc = RC::SendFailed;
    }
    if (rc == RC::Okay && !rf95->waitPacketSent()) {
        rc = RC::TimedOut;
    }
#endif

    return rc;
}

RC wait_recv(uint8_t buf[], uint8_t& len, uint32_t timeout_ms) {
    RC rc = RC::Okay;
    if (!is_active()) {
        rc = init();
    }
#ifdef SIMULATE
    if (timeout_ms != 0) {
        struct timeval start, now;
        gettimeofday(&start, NULL);

        while (1) {
            ssize_t nread = read(RF95, buf, len);
            if (nread > 0) {
                len = (uint8_t)nread;
                break;
            }

            gettimeofday(&now, NULL);
            long elapsed_ms = (now.tv_sec - start.tv_sec) * 1000L +
                              (now.tv_usec - start.tv_usec) / 1000L;

            if (elapsed_ms >= timeout_ms) {
                rc = RC::RecvFailed;
                break;
            }
        }
    } else {
        ssize_t nread = read(RF95, buf, len);
        if (nread <= 0) {
            rc = RC::RecvFailed;
        } else {
            len = (uint8_t)nread;
        }
    }
#else
    if (timeout_ms == 0) {
        RF95.waitAvailable();
    } else {
        RF95.waitAvailableTimeout(timeout_ms);
    }

    if (rc == RC::Okay && !RF95.recv(buf, &len)) {
        rc = RC::RecvFailed;
    }
#endif

    return rc;
}

}  // namespace lora
