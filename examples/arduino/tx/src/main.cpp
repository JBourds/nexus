#include "LoraRadio.h"
#ifdef SIMULATE
#include <stdint.h>
#include <unistd.h>

#include <iostream>
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
    Serial.begin(9600);
    while (!Serial) {
    }
#endif
    if (lora::init() != RC::Okay) {
        error("Lora init");
    }
}

void loop() {
    char msg[lora::PACKET_MAX_SIZE_BYTES];
    static uint32_t counter = 0;
    size_t nwritten =
        snprintf(msg, lora::PACKET_MAX_SIZE_BYTES, "TX[%lu]", counter++);
    RC rc = lora::send(reinterpret_cast<uint8_t*>(msg), nwritten);
    if (rc == lora::RC::Okay) {
        print(reinterpret_cast<char*>(msg));
    } else {
        error("Failed to send message");
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
