#pragma once
#ifndef SIMULATE
#include <RH_RF95.h>
#endif

#include <stddef.h>
#include <stdint.h>

namespace lora {

// This is the maximum size in bytes allowed for the data section of the
const size_t PACKET_MAX_SIZE_BYTES = 251;

enum struct RC : uint8_t {
    Okay,
    AlreadyInit,
    NotInit,
    InitFailed,
    DeinitFailed,
    SetFrequencyFailed,
    SdActive,
    FailedToDeinitSd,
    FailedToRestoreSd,
    SendFailed,
    RecvFailed,
    TimedOut,
};

bool is_active();

#ifdef SIMULATE
int* get();
#else
RH_RF95* get();
#endif

int16_t last_rssi();

RC init();

RC deinit();

/**
 * Wrapper function for the actual library implementation. Uses internal buffer.
 *
 * @param buf: Send buffer.
 * @param sz: Remaining size in buffer.
 *
 * @returns (RC): Return code.
 */
RC send(const uint8_t buf[], size_t sz);

/**
 * Wrapper function for the actual library implementation.
 *
 * @param buf: Receive buffer.
 * @param len: Total size of buffer. Gets set to number of bytes copied.
 * @param timeout_ms: Optional timeout in milliseconds to pass to the RF95
 * library. If equal to 0, blocks until message is received.
 *
 * @returns (RC): Return code.
 */
RC wait_recv(uint8_t buf[], uint8_t& len, uint32_t timeout_ms);

}  // namespace lora
