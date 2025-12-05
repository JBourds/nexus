#include "LoraRadio.h"
#ifdef SIMULATE
#include <iostream>
#else
#include <Wire.h>
#include <Arduino.h>
#endif

using lora::RC;

void error(const char* msg) {
#ifdef SIMULATE
    std::cerr << "ERROR: " << msg << std::endl;
    exit(EXIT_FAILURE);
#else
    Serial.print("ERROR: ");
    Serial.println(msg);
    while (true) {
    }
#endif
}

void print(const char* msg) {
#ifdef SIMULATE
    std::cout << msg << std::endl;
#else
    Serial.println(msg);
#endif
}

void setup() {
#ifndef SIMULATE
    Wire.begin();
    Serial.begin(9600);
    while (!Serial) {
    }
    delay(50);
#endif
    RC rc = lora::init();
    switch (rc) {
        case RC::InitFailed:
            error("Failed to initialized RF95");
            break;
        case RC::SetFrequencyFailed:
            error("Failed to set frequency");
            break;
        default:
            break;
    }
}

void loop() {
    char msg[lora::PACKET_MAX_SIZE_BYTES];
    uint8_t len = lora::PACKET_MAX_SIZE_BYTES;
    RC rc = lora::wait_recv(reinterpret_cast<uint8_t*>(msg), len, 5000);
    if (rc == lora::RC::Okay) {
        print(msg);
    } else if (rc == lora::RC::TimedOut) {
        print("Timed out");
    } else {
        error("Failed to receive message");
    }
}

#ifdef SIMULATE
int main() {
    setup();
    while (true) {
        loop();
    }
}
#endif
