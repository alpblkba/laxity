/* see include/qos/telemetry.h for the wire format this implements and for what it promises. */

#include "qos/telemetry.h"

#include <stdatomic.h>
#include <string.h>

/* power of two so the index wrap is a mask. thirty two records is 1024 bytes of bss and covers the producer running ahead of an exporter that is blocked on a UART transmit, which at 921600 baud is a few milliseconds. */
#define QOS_RING_CAP   32u
#define QOS_RING_MASK  (QOS_RING_CAP - 1u)

static qos_infer_record_t s_ring[QOS_RING_CAP];

/* free running indices rather than wrapped ones, so head minus tail is the occupancy and the empty and full cases do not collide. single producer writes head, single consumer writes tail, and the release and acquire pair is what publishes the record body before the index that exposes it. */
static _Atomic uint32_t s_head;
static _Atomic uint32_t s_tail;
static _Atomic uint32_t s_seq_next;
static _Atomic uint32_t s_dropped;

static uint32_t s_clock_hz;
static uint32_t s_cyccnt_hz;
static uint8_t  s_header_flags;

static uint32_t s_null_read_median;
static uint32_t s_null_read_p99;
static uint32_t s_null_push_median;
static uint32_t s_null_push_p99;
static uint16_t s_null_n;

static const qos_placement_t *s_placements;
static uint8_t s_placement_count;

static size_t header_payload_len(void)
{
    return (size_t)QOS_HEADER_FIXED + (size_t)s_placement_count * QOS_PLACEMENT_SIZE;
}

/* CRC-16/CCITT-FALSE: polynomial 0x1021, initial value 0xFFFF, no reflection, no final xor.
 *
 * bitwise rather than table driven, which is 8 iterations per byte and about 3000 cycles for a full batch. the ceiling is wire bandwidth on the export path, which is best effort and off the measured window, so a 512 byte table would buy nothing here. add one if the exporter ever becomes the bottleneck. */
static uint16_t crc16(const uint8_t *data, size_t len)
{
    uint16_t crc = 0xFFFFu;

    for (size_t i = 0u; i < len; ++i) {
        crc ^= (uint16_t)((uint16_t)data[i] << 8);
        for (int bit = 0; bit < 8; ++bit) {
            crc = (crc & 0x8000u) ? (uint16_t)((uint16_t)(crc << 1) ^ 0x1021u)
                                  : (uint16_t)(crc << 1);
        }
    }
    return crc;
}

static void put32(uint8_t *p, uint32_t v)
{
    p[0] = (uint8_t)v;
    p[1] = (uint8_t)(v >> 8);
    p[2] = (uint8_t)(v >> 16);
    p[3] = (uint8_t)(v >> 24);
}

/* write the eight frame bytes in front of a payload that is already sitting at buf + QOS_FRAME_OVERHEAD. building the payload in place avoids a second buffer and a copy, and the frame header is written byte by byte so no struct packing assumption reaches the wire. */
static size_t frame(uint8_t *buf, uint8_t type, uint16_t len)
{
    uint16_t crc = crc16(buf + QOS_FRAME_OVERHEAD, len);

    buf[0] = QOS_TELEMETRY_MAGIC0;
    buf[1] = QOS_TELEMETRY_MAGIC1;
    buf[2] = (uint8_t)QOS_TELEMETRY_VERSION;
    buf[3] = type;
    buf[4] = (uint8_t)len;
    buf[5] = (uint8_t)(len >> 8);
    buf[6] = (uint8_t)crc;
    buf[7] = (uint8_t)(crc >> 8);

    return (size_t)QOS_FRAME_OVERHEAD + len;
}

void qos_telemetry_init(uint32_t clock_hz, uint32_t cyccnt_hz, uint8_t header_flags)
{
    s_clock_hz = clock_hz;
    s_cyccnt_hz = cyccnt_hz;
    s_header_flags = header_flags;
    qos_telemetry_reset();
}

void qos_telemetry_set_null_probe(uint32_t read_median, uint32_t read_p99,
                                  uint32_t push_median, uint32_t push_p99, uint16_t n)
{
    s_null_read_median = read_median;
    s_null_read_p99 = read_p99;
    s_null_push_median = push_median;
    s_null_push_p99 = push_p99;
    s_null_n = n;
}

void qos_telemetry_set_placements(const qos_placement_t *table, uint8_t count)
{
    s_placements = table;
    s_placement_count = (table == NULL) ? 0u : count;
}

void qos_telemetry_reset(void)
{
    atomic_store_explicit(&s_head, 0u, memory_order_relaxed);
    atomic_store_explicit(&s_tail, 0u, memory_order_relaxed);
    atomic_store_explicit(&s_seq_next, 0u, memory_order_relaxed);
    atomic_store_explicit(&s_dropped, 0u, memory_order_release);
}

bool qos_telemetry_push(const qos_infer_record_t *rec)
{
    if (rec == NULL) {
        return false;
    }

    uint32_t head = atomic_load_explicit(&s_head, memory_order_relaxed);
    uint32_t tail = atomic_load_explicit(&s_tail, memory_order_acquire);
    uint32_t seq  = atomic_fetch_add_explicit(&s_seq_next, 1u, memory_order_relaxed);

    /* seq advances whether or not the record survives, so a gap in the parsed sequence is a drop and the reader does not need the header to see one. */
    if ((head - tail) >= QOS_RING_CAP) {
        atomic_fetch_add_explicit(&s_dropped, 1u, memory_order_relaxed);
        return false;
    }

    s_ring[head & QOS_RING_MASK] = *rec;
    s_ring[head & QOS_RING_MASK].seq = seq;
    atomic_store_explicit(&s_head, head + 1u, memory_order_release);
    return true;
}

size_t qos_telemetry_drain(uint8_t *buf, size_t cap)
{
    const size_t minimum = (size_t)QOS_FRAME_OVERHEAD + header_payload_len() +
                           (size_t)QOS_FRAME_OVERHEAD + QOS_RECORD_SIZE;

    if (buf == NULL || cap < minimum) {
        return 0u;
    }

    uint32_t tail = atomic_load_explicit(&s_tail, memory_order_relaxed);
    uint32_t head = atomic_load_explicit(&s_head, memory_order_acquire);
    if (head == tail) {
        return 0u;
    }

    uint8_t *payload = buf + QOS_FRAME_OVERHEAD;
    put32(payload + 0,  s_clock_hz);
    put32(payload + 4,  s_cyccnt_hz);
    put32(payload + 8,  atomic_load_explicit(&s_seq_next, memory_order_relaxed));
    put32(payload + 12, atomic_load_explicit(&s_dropped, memory_order_relaxed));
    payload[16] = s_header_flags;
    payload[17] = s_placement_count;
    payload[18] = 0u;  /* model table, not populated yet */
    payload[19] = (uint8_t)QOS_RECORD_SIZE;
    put32(payload + 20, s_null_read_median);
    put32(payload + 24, s_null_read_p99);
    put32(payload + 28, s_null_push_median);
    put32(payload + 32, s_null_push_p99);
    payload[36] = (uint8_t)s_null_n;
    payload[37] = (uint8_t)(s_null_n >> 8);
    payload[38] = 0u;
    payload[39] = 0u;

    /* the entries are plain old data with the same layout on the wire as in memory, checked by the static assertion in the header, so they copy rather than serialise field by field. */
    for (uint8_t i = 0u; i < s_placement_count; ++i) {
        memcpy(payload + QOS_HEADER_FIXED + (size_t)i * QOS_PLACEMENT_SIZE,
               &s_placements[i], QOS_PLACEMENT_SIZE);
    }

    size_t used = frame(buf, QOS_FRAME_HEADER, (uint16_t)header_payload_len());

    uint32_t room  = (uint32_t)((cap - used - QOS_FRAME_OVERHEAD) / QOS_RECORD_SIZE);
    uint32_t count = head - tail;
    if (count > room) {
        count = room;
    }

    uint8_t *body = buf + used + QOS_FRAME_OVERHEAD;
    for (uint32_t i = 0u; i < count; ++i) {
        memcpy(body + (size_t)i * QOS_RECORD_SIZE,
               &s_ring[(tail + i) & QOS_RING_MASK], QOS_RECORD_SIZE);
    }

    used += frame(buf + used, QOS_FRAME_BATCH, (uint16_t)(count * QOS_RECORD_SIZE));

    /* the slots are only released after their contents have been copied out, so a producer that fills the ring during this loop cannot overwrite a record that is still being read. */
    atomic_store_explicit(&s_tail, tail + count, memory_order_release);
    return used;
}

bool qos_telemetry_has_stall_attribution(void)
{
    return (s_header_flags & QOS_HDR_STALL_AVAILABLE) != 0u;
}
