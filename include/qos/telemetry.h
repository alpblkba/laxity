/* the on-wire telemetry contract, implemented by runtime/telemetry.c and parsed by tools/telemetry_parse.py.
 *
 * this header is the format. changing anything here changes what a recorded stream means, so it comes with a bump of QOS_TELEMETRY_VERSION and a matching change in the host reader.
 *
 * nothing here includes a vendor header. the cycle values arrive from the caller, so this file has no opinion about where they came from and builds on the host for the tests under tools/host-tests.
 */
#ifndef QOS_TELEMETRY_H
#define QOS_TELEMETRY_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define QOS_TELEMETRY_VERSION  2u

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
#define QOS_HEADER_FIXED       40u  /* the part of a header payload that is always present */
#define QOS_PLACEMENT_SIZE     20u  /* one placement entry, n_regions of them follow the fixed part */
#define QOS_RECORD_SIZE        32u

/* header frame flags. two bits rather than one, since the capability and its use are different facts: this part was measured to implement the DWT profiling counters, and nothing populates stall_cyc from them yet. a reader that saw only the capability bit could not tell an absent counter from an unimplemented one, and every record would look the same in both cases. */
#define QOS_HDR_STALL_AVAILABLE  (1u << 0)  /* the port can attribute stall cycles */
#define QOS_HDR_STALL_POPULATED  (1u << 1)  /* this stream actually carries them in stall_cyc */

/* one placement label the run measured, and where its arena actually sat.
 *
 * the address is recorded per label because the arena has no fixed home: adding unrelated bss moves it, and it has already moved by 0x2c8 across two builds of the same firmware. a capture that carried only a region name would not be able to show that the buffer landed where the firmware meant to put it.
 *
 * a control entry is the same memory under a second identity rather than a second region, so its arena address matches the entry it shadows. an alternate address entry is the other half of that argument: the same region at a different address, which is what tells a difference between regions apart from a difference between addresses. */
#define QOS_PLACEMENT_CONTROL  (1u << 0)
#define QOS_PLACEMENT_ALT_ADDR (1u << 1)

typedef struct {
    uint8_t  id;          /* matches qos_infer_record_t::region_id */
    uint8_t  flags;
    uint16_t rel_cost;
    uint32_t arena_addr;
    uint32_t arena_size;
    char     name[8];     /* NUL padded, not NUL terminated when the name fills it */
} qos_placement_t;

_Static_assert(sizeof(qos_placement_t) == QOS_PLACEMENT_SIZE, "placement entry must be 20 bytes on the wire");

/* set when the measured window crossed a CYCCNT wrap, which is the case where a host recomputing the duration from raw counter values would be wrong. qos_cyc_delta() is already correct across one wrap, so the flag is a warning about interpretation rather than about the value. */
#define QOS_FLAG_CYCCNT_WRAP   (1u << 0)

/* one completed activation. plain old data, 32 bytes, offsets fixed by TELEMETRY.md.
 *
 * release_cyc is a raw CYCCNT reading taken at release and it wraps every 26.8 seconds at the measured clock. exec_cyc, cpu_cyc and stall_cyc are durations in cycles. stall_cyc is zero when the port reports no stall attribution, and the header frame says which case applies so a reader does not have to infer it from zeros.
 *
 * seq is assigned by qos_telemetry_push() rather than by the caller, and it advances on records the ring dropped as well as on records it accepted, so a gap in seq is a drop that is visible per record. */
/* aggressor_idx packs which competing bus master was running during the measured window: the low
 * byte is the memory region its buffers sit in, using the same ids as region_id, and zero means
 * no aggressor. the high byte is an index into the footprint table the run declares.
 *
 * reserved carries the aggressor's completed transfer count at the end of the window. a channel
 * that failed to start is indistinguishable from a channel that caused no contention, so a count
 * that does not advance between consecutive records of an aggressor bearing cell marks the run
 * as broken rather than null. it is a lower bound, since several transfers can complete between
 * two samples and be counted once. */
#define QOS_AGGRESSOR_REGION(idx)    ((uint8_t)((idx) & 0xFFu))
#define QOS_AGGRESSOR_FOOTPRINT(idx) ((uint8_t)(((idx) >> 8) & 0xFFu))

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

/* what the measurement cost, copied into every header frame. every reported result carries the null probe overhead beside it, and carrying it in the stream rather than in a log line keeps a raw capture self contained. */
void qos_telemetry_set_null_probe(uint32_t read_median, uint32_t read_p99,
                                  uint32_t push_median, uint32_t push_p99, uint16_t n);

/* the placement labels this run measured. the table is not copied, so it has to outlive the exporter, which on the target means static storage. at most 255 entries, since n_regions is one byte. */
void qos_telemetry_set_placements(const qos_placement_t *table, uint8_t count);

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
