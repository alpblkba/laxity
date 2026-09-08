/* the on-wire telemetry contract, implemented by runtime/telemetry.c and parsed by tools/telemetry-parse.py.
 *
 * self-docs/docs/TELEMETRY.md is the specification and this header is its C form. changing either means changing both in the same commit and bumping QOS_TELEMETRY_VERSION.
 *
 * nothing here includes a vendor header. the cycle values arrive from the caller, so this file has no opinion about where they came from and builds on the host for the tests under tools/host-tests.
 */
#ifndef QOS_TELEMETRY_H
#define QOS_TELEMETRY_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define QOS_TELEMETRY_VERSION  1u

/* records are streamed verbatim, so the struct layout is the wire layout and a big endian target would emit a different format under the same version number. both the STM32U585 and the macOS arm64 host are little endian, so this is a contract rather than a portability gap to close now. */
#if defined(__BYTE_ORDER__) && (__BYTE_ORDER__ != __ORDER_LITTLE_ENDIAN__)
#error "qos telemetry streams records verbatim and the format is little endian"
#endif

/* framing. the magic is "LX" in ASCII, which is two bytes that also occur in ordinary text, so a reader syncs on the magic and then validates the CRC before accepting anything. */
#define QOS_TELEMETRY_MAGIC0   0x4Cu
#define QOS_TELEMETRY_MAGIC1   0x58u

#define QOS_FRAME_HEADER       0u
#define QOS_FRAME_BATCH        1u

#define QOS_FRAME_OVERHEAD     8u   /* magic, version, type, payload_len, crc16 */
#define QOS_HEADER_PAYLOAD     20u
#define QOS_RECORD_SIZE        32u

/* header frame flags. two bits rather than one, since the capability and its use are different facts: block 3 measured that this part implements the DWT profiling counters, and nothing populates stall_cyc from them until the RQ2 attribution work lands. a reader that saw only the capability bit could not tell an absent counter from an unimplemented one, and every record would look the same in both cases. */
#define QOS_HDR_STALL_AVAILABLE  (1u << 0)  /* the port can attribute stall cycles */
#define QOS_HDR_STALL_POPULATED  (1u << 1)  /* this stream actually carries them in stall_cyc */

/* set when the measured window crossed a CYCCNT wrap, which is the case where a host recomputing the duration from raw counter values would be wrong. qos_cyc_delta() is already correct across one wrap, so the flag is a warning about interpretation rather than about the value. */
#define QOS_FLAG_CYCCNT_WRAP   (1u << 0)

/* one completed activation. plain old data, 32 bytes, offsets fixed by TELEMETRY.md.
 *
 * release_cyc is a raw CYCCNT reading taken at release and it wraps every 26.8 seconds at the measured clock. exec_cyc, cpu_cyc and stall_cyc are durations in cycles. stall_cyc is zero when the port reports no stall attribution, and the header frame says which case applies so a reader does not have to infer it from zeros.
 *
 * seq is assigned by qos_telemetry_push() rather than by the caller, and it advances on records the ring dropped as well as on records it accepted, so a gap in seq is a drop that is visible per record. */
typedef struct {
    uint32_t seq;
    uint32_t release_cyc;
    uint32_t exec_cyc;
    uint32_t cpu_cyc;
    uint32_t stall_cyc;
    uint16_t model_id;
    uint8_t  region_id;
    uint8_t  flags;
    uint16_t aggressor_idx;
    uint16_t padding;
    uint32_t reserved;
} qos_infer_record_t;

_Static_assert(sizeof(qos_infer_record_t) == QOS_RECORD_SIZE, "record must be 32 bytes on the wire");
_Static_assert(offsetof(qos_infer_record_t, release_cyc) == 4, "record layout moved");
_Static_assert(offsetof(qos_infer_record_t, exec_cyc) == 8, "record layout moved");
_Static_assert(offsetof(qos_infer_record_t, cpu_cyc) == 12, "record layout moved");
_Static_assert(offsetof(qos_infer_record_t, stall_cyc) == 16, "record layout moved");
_Static_assert(offsetof(qos_infer_record_t, model_id) == 20, "record layout moved");
_Static_assert(offsetof(qos_infer_record_t, region_id) == 22, "record layout moved");
_Static_assert(offsetof(qos_infer_record_t, flags) == 23, "record layout moved");
_Static_assert(offsetof(qos_infer_record_t, aggressor_idx) == 24, "record layout moved");
_Static_assert(offsetof(qos_infer_record_t, reserved) == 28, "record layout moved");

/* record the run metadata the target knows. clock_hz is what the clock tree is configured for and cyccnt_hz is what qos_dwt_measure_hz() observed, and keeping both lets a capture show a configuration that did not take. header_flags is the QOS_HDR_ set that goes into every header frame. */
void qos_telemetry_init(uint32_t clock_hz, uint32_t cyccnt_hz, uint8_t header_flags);

/* empty the ring and clear the sequence and drop counters. the producer owns this call, and the consumer must not be draining concurrently, which on the target means calling it before the exporter thread is released. */
void qos_telemetry_reset(void);

/* producer side. copies the record, assigns its seq, and returns false when the ring was full, in which case the record is dropped and counted. export is best effort because measurement must not become the reason a deadline is missed. */
bool qos_telemetry_push(const qos_infer_record_t *rec);

/* consumer side. writes a header frame followed by a record batch frame into buf and returns the byte count, or zero when the ring is empty or cap is too small for both frames.
 *
 * the header goes in front of every batch so no capture can open on records without their metadata. that costs 28 bytes against a batch of up to cap, which at 921600 baud is not a bandwidth this project runs out of. */
size_t qos_telemetry_drain(uint8_t *buf, size_t cap);

/* QOS_HDR_STALL_AVAILABLE as the header frame reports it, exposed so the caller does not have to parse its own stream. it says the port can attribute, not that stall_cyc is filled in. */
bool qos_telemetry_has_stall_attribution(void);

#endif /* QOS_TELEMETRY_H */
