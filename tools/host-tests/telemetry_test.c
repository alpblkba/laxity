/* host checks for the telemetry ring, the frame layout and the CYCCNT wrap helper, plus the two streams tools/host-tests/run.sh feeds to the parser.
 *
 * assert based and framework free on purpose. the thing being checked is a wire format and a lock free ring, and both are small enough that a test framework would be larger than the code under test.
 *
 * it also writes the streams the Python parser reads, which is the point of running it here rather than only on the target: the parser and the firmware are two implementations of one document, and a stream that only ever round trips through its own writer proves nothing about the format.
 */

#include "qos/telemetry.h"
#include "counters_dwt.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int failures;

#define CHECK(cond) \
    do { \
        if (!(cond)) { \
            printf("FAIL %s:%d  %s\n", __FILE__, __LINE__, #cond); \
            ++failures; \
        } \
    } while (0)

#define CHECK_EQ(got, want) \
    do { \
        unsigned long g_ = (unsigned long)(got), w_ = (unsigned long)(want); \
        if (g_ != w_) { \
            printf("FAIL %s:%d  %s = %lu, expected %lu\n", __FILE__, __LINE__, #got, g_, w_); \
            ++failures; \
        } \
    } while (0)

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

static uint16_t rd16(const uint8_t *p) { return (uint16_t)(p[0] | ((uint16_t)p[1] << 8)); }
static uint32_t rd32(const uint8_t *p)
{
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}

/* validate one frame in place and return its total length, or zero when it is not a frame. */
static size_t check_frame(const uint8_t *p, size_t avail, uint8_t type)
{
    if (avail < QOS_FRAME_OVERHEAD) { return 0u; }
    CHECK_EQ(p[0], QOS_TELEMETRY_MAGIC0);
    CHECK_EQ(p[1], QOS_TELEMETRY_MAGIC1);
    CHECK_EQ(p[2], QOS_TELEMETRY_VERSION);
    CHECK_EQ(p[3], type);

    uint16_t len = rd16(p + 4);
    if ((size_t)len + QOS_FRAME_OVERHEAD > avail) { CHECK(0); return 0u; }
    CHECK_EQ(rd16(p + 6), crc16(p + QOS_FRAME_OVERHEAD, len));
    return (size_t)QOS_FRAME_OVERHEAD + len;
}

static void test_record_layout(void)
{
    /* the static asserts in the header cover the offsets. this checks that a record reaches the wire as those offsets describe, since streaming verbatim is the whole reason the offsets are fixed. */
    qos_infer_record_t rec = {0};
    rec.seq = 0x11223344u;
    rec.release_cyc = 0x55667788u;
    rec.exec_cyc = 0x99AABBCCu;
    rec.cpu_cyc = 0xDDEEFF00u;
    rec.stall_cyc = 0x0F1E2D3Cu;
    rec.model_id = 0xBEEFu;
    rec.region_id = 0x5Au;
    rec.flags = QOS_FLAG_CYCCNT_WRAP;
    rec.aggressor_idx = 0xCAFEu;
    rec.reserved = 0xA5A5A5A5u;

    uint8_t wire[QOS_RECORD_SIZE];
    memcpy(wire, &rec, sizeof wire);

    CHECK_EQ(rd32(wire + 0), 0x11223344u);
    CHECK_EQ(rd32(wire + 4), 0x55667788u);
    CHECK_EQ(rd32(wire + 8), 0x99AABBCCu);
    CHECK_EQ(rd32(wire + 12), 0xDDEEFF00u);
    CHECK_EQ(rd32(wire + 16), 0x0F1E2D3Cu);
    CHECK_EQ(rd16(wire + 20), 0xBEEFu);
    CHECK_EQ(wire[22], 0x5Au);
    CHECK_EQ(wire[23], QOS_FLAG_CYCCNT_WRAP);
    CHECK_EQ(rd16(wire + 24), 0xCAFEu);
    CHECK_EQ(rd32(wire + 28), 0xA5A5A5A5u);
}

static void test_wrap(void)
{
    /* 26.8 seconds of counting at 159,999,450 Hz fits in 32 bits and the next cycle does not, so a thirty repetition sweep crosses this boundary as a matter of course rather than as an edge case. */
    CHECK_EQ(qos_cyc_delta(0xFFFFFFF0u, 0x0000000Fu), 0x1Fu);
    CHECK_EQ(qos_cyc_delta(0x00000010u, 0x00000030u), 0x20u);
    CHECK_EQ(qos_cyc_delta(0xFFFFFFFFu, 0xFFFFFFFFu), 0u);

    CHECK(qos_cyc_wrapped(0xFFFFFFF0u, 0x0000000Fu));
    CHECK(!qos_cyc_wrapped(0x00000010u, 0x00000030u));
    CHECK(!qos_cyc_wrapped(0x00000010u, 0x00000010u));

    /* two wraps are out of contract and the helper reports the interval as if one had happened, which is stated here so a later reader does not mistake the behaviour for a guarantee. */
    CHECK_EQ(qos_cyc_delta(0x00000000u, 0x00000005u), 5u);
}

static void test_round_trip(void)
{
    uint8_t buf[2048];
    qos_infer_record_t rec = {0};

    qos_telemetry_init(160000000u, 159999450u, QOS_HDR_STALL_AVAILABLE);
    CHECK(qos_telemetry_has_stall_attribution());
    CHECK_EQ(qos_telemetry_drain(buf, sizeof buf), 0u);

    for (uint32_t i = 0u; i < 5u; ++i) {
        rec.seq = 0xDEADBEEFu;  /* overwritten by the ring, which owns the sequence */
        rec.exec_cyc = 1000u + i;
        CHECK(qos_telemetry_push(&rec));
    }

    size_t n = qos_telemetry_drain(buf, sizeof buf);
    size_t head = check_frame(buf, n, QOS_FRAME_HEADER);
    CHECK_EQ(head, (size_t)QOS_FRAME_OVERHEAD + QOS_HEADER_PAYLOAD);

    const uint8_t *hp = buf + QOS_FRAME_OVERHEAD;
    CHECK_EQ(rd32(hp + 0), 160000000u);
    CHECK_EQ(rd32(hp + 4), 159999450u);
    CHECK_EQ(rd32(hp + 8), 5u);
    CHECK_EQ(rd32(hp + 12), 0u);
    CHECK_EQ(hp[16], 1u);
    CHECK_EQ(hp[17], 0u);
    CHECK_EQ(hp[18], 0u);
    CHECK_EQ(hp[19], QOS_RECORD_SIZE);

    size_t batch = check_frame(buf + head, n - head, QOS_FRAME_BATCH);
    CHECK_EQ(batch, (size_t)QOS_FRAME_OVERHEAD + 5u * QOS_RECORD_SIZE);
    CHECK_EQ(head + batch, n);

    const uint8_t *body = buf + head + QOS_FRAME_OVERHEAD;
    for (uint32_t i = 0u; i < 5u; ++i) {
        CHECK_EQ(rd32(body + i * QOS_RECORD_SIZE + 0), i);
        CHECK_EQ(rd32(body + i * QOS_RECORD_SIZE + 8), 1000u + i);
    }

    CHECK_EQ(qos_telemetry_drain(buf, sizeof buf), 0u);
}

static void test_partial_drain(void)
{
    /* a buffer that holds both frames but only two records has to take two and leave the rest, since the exporter's buffer is what limits a batch rather than the ring. */
    uint8_t buf[QOS_FRAME_OVERHEAD + QOS_HEADER_PAYLOAD + QOS_FRAME_OVERHEAD + 2u * QOS_RECORD_SIZE];
    qos_infer_record_t rec = {0};

    qos_telemetry_reset();
    for (uint32_t i = 0u; i < 5u; ++i) { CHECK(qos_telemetry_push(&rec)); }

    CHECK_EQ(qos_telemetry_drain(buf, sizeof buf), sizeof buf);
    CHECK_EQ(rd16(buf + QOS_FRAME_OVERHEAD + QOS_HEADER_PAYLOAD + 4), 2u * QOS_RECORD_SIZE);

    CHECK_EQ(qos_telemetry_drain(buf, sizeof buf), sizeof buf);
    CHECK_EQ(rd16(buf + QOS_FRAME_OVERHEAD + QOS_HEADER_PAYLOAD + 4), 2u * QOS_RECORD_SIZE);
    CHECK(qos_telemetry_drain(buf, sizeof buf) > 0u);
    CHECK_EQ(rd16(buf + QOS_FRAME_OVERHEAD + QOS_HEADER_PAYLOAD + 4), 1u * QOS_RECORD_SIZE);
    CHECK_EQ(qos_telemetry_drain(buf, sizeof buf), 0u);

    /* too small for a header and one record together, so nothing is emitted rather than a batch without its header. */
    CHECK_EQ(qos_telemetry_drain(buf, 32u), 0u);
}

static void test_overflow(void)
{
    uint8_t buf[2048];
    qos_infer_record_t rec = {0};

    qos_telemetry_reset();
    for (uint32_t i = 0u; i < 32u; ++i) { CHECK(qos_telemetry_push(&rec)); }
    /* the thirty third does not fit, and the sequence still advances so the gap it leaves is visible in the parsed stream without consulting the header. */
    for (uint32_t i = 0u; i < 4u; ++i) { CHECK(!qos_telemetry_push(&rec)); }

    size_t n = qos_telemetry_drain(buf, sizeof buf);
    const uint8_t *hp = buf + QOS_FRAME_OVERHEAD;
    CHECK_EQ(rd32(hp + 8), 36u);
    CHECK_EQ(rd32(hp + 12), 4u);

    size_t head = (size_t)QOS_FRAME_OVERHEAD + QOS_HEADER_PAYLOAD;
    CHECK_EQ(rd16(buf + head + 4), 32u * QOS_RECORD_SIZE);
    CHECK_EQ(n, head + QOS_FRAME_OVERHEAD + 32u * QOS_RECORD_SIZE);

    const uint8_t *body = buf + head + QOS_FRAME_OVERHEAD;
    CHECK_EQ(rd32(body + 0), 0u);
    CHECK_EQ(rd32(body + 31u * QOS_RECORD_SIZE), 31u);

    CHECK(qos_telemetry_push(&rec));
    n = qos_telemetry_drain(buf, sizeof buf);
    CHECK_EQ(rd32(buf + head + QOS_FRAME_OVERHEAD), 36u);
}

/* the mixed stream the firmware actually produces: a status line that contains the two magic bytes, then framed telemetry, then more of the same. */
static const char NOISE[] = "infer rc=0 class=1(Stationary) MATCH LX opt=-O0\r\n";

static void write_streams(const char *dir)
{
    uint8_t buf[2048];
    uint8_t stream[8192];
    size_t len = 0u;
    size_t first_header_payload = 0u;
    qos_infer_record_t rec = {0};

    qos_telemetry_init(160000000u, 159999450u, QOS_HDR_STALL_AVAILABLE);

    memcpy(stream + len, NOISE, sizeof NOISE - 1u); len += sizeof NOISE - 1u;

    for (uint32_t i = 0u; i < 32u; ++i) {
        rec.exec_cyc = 300000u + i;
        rec.flags = ((i % 8u) == 0u) ? QOS_FLAG_CYCCNT_WRAP : 0u;
        (void)qos_telemetry_push(&rec);
    }
    rec.flags = 0u;
    for (uint32_t i = 0u; i < 4u; ++i) { (void)qos_telemetry_push(&rec); }

    first_header_payload = len + QOS_FRAME_OVERHEAD;
    size_t n = qos_telemetry_drain(buf, sizeof buf);
    memcpy(stream + len, buf, n); len += n;

    memcpy(stream + len, NOISE, sizeof NOISE - 1u); len += sizeof NOISE - 1u;

    for (uint32_t i = 0u; i < 4u; ++i) { (void)qos_telemetry_push(&rec); }
    n = qos_telemetry_drain(buf, sizeof buf);
    memcpy(stream + len, buf, n); len += n;

    memcpy(stream + len, NOISE, sizeof NOISE - 1u); len += sizeof NOISE - 1u;

    char path[512];
    snprintf(path, sizeof path, "%s/stream.bin", dir);
    FILE *f = fopen(path, "wb");
    if (f == NULL) { printf("FAIL cannot write %s\n", path); ++failures; return; }
    fwrite(stream, 1u, len, f);
    fclose(f);

    /* one byte of the first header payload flipped. the frame then fails its CRC, the parser
       resumes one byte later, and the batch behind it arrives with no header in front of it,
       which is the case docs/TELEMETRY.md says a reader must discard rather than parse. */
    stream[first_header_payload + 19u] ^= 0xFFu;
    snprintf(path, sizeof path, "%s/corrupt.bin", dir);
    f = fopen(path, "wb");
    if (f == NULL) { printf("FAIL cannot write %s\n", path); ++failures; return; }
    fwrite(stream, 1u, len, f);
    fclose(f);
}

int main(int argc, char **argv)
{
    test_record_layout();
    test_wrap();
    test_round_trip();
    test_partial_drain();
    test_overflow();

    if (argc > 1) { write_streams(argv[1]); }

    if (failures == 0) {
        printf("host tests ok\n");
        return 0;
    }
    printf("%d host test failures\n", failures);
    return 1;
}
