#define _POSIX_C_SOURCE 200809L

#include "qos/telemetry.h"
#include "scenarios.h"

#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <netinet/in.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <time.h>
#include <unistd.h>

#define LAXITY_SIM_LINE_CAP   512u
#define LAXITY_SIM_TOKEN_CAP  16u
#define LAXITY_SIM_DRAIN_CAP  512u
#define LAXITY_SIM_ORIGIN_FLAG (1u << 2)

typedef struct {
    const char *scenario_arg;
    const char *udp_arg;
    const char *output_path;
    uint32_t seed;
} options_t;

typedef struct {
    FILE *file;
    int socket_fd;
    struct sockaddr_in udp_addr;
    bool has_udp;
    uint32_t pace_ms;
    bool corrupt_next;
    bool gap_next;
    uint64_t bytes_written;
    uint64_t buffers_written;
} sink_t;

typedef struct {
    sink_t sink;
    uint32_t rng;
    uint32_t clock_hz;
    uint32_t cyccnt_hz;
    uint32_t release_cyc;
    uint32_t period_cyc;
    uint32_t transfer_count;
    uint64_t attempted_records;
    uint64_t accepted_records;
    uint64_t rejected_records;
} simulator_t;

static void usage(FILE *stream)
{
    fprintf(stream,
            "usage: laxity-sim --scenario PATH|NAME [--seed N] [--udp IPV4:PORT] [--output PATH]\n"
            "\n"
            "NAME resolves to scenarios/NAME.lxs from the repository root.\n"
            "At least one of --udp and --output is required. Output files are never overwritten.\n");
}

static bool parse_u32(const char *text, uint32_t *out)
{
    char *end = NULL;
    unsigned long long value;

    if (text == NULL || text[0] == '\0' || text[0] == '-') {
        return false;
    }
    errno = 0;
    value = strtoull(text, &end, 10);
    if (errno != 0 || end == text || *end != '\0' || value > UINT32_MAX) {
        return false;
    }
    *out = (uint32_t)value;
    return true;
}

static bool parse_options(int argc, char **argv, options_t *options)
{
    bool seed_seen = false;

    memset(options, 0, sizeof *options);
    options->seed = 1u;

    for (int i = 1; i < argc; ++i) {
        const char *arg = argv[i];
        if (strcmp(arg, "-h") == 0 || strcmp(arg, "--help") == 0) {
            usage(stdout);
            exit(0);
        }
        if (i + 1 >= argc) {
            fprintf(stderr, "laxity-sim: %s requires a value\n", arg);
            return false;
        }
        if (strcmp(arg, "--scenario") == 0 && options->scenario_arg == NULL) {
            options->scenario_arg = argv[++i];
        } else if (strcmp(arg, "--seed") == 0 && !seed_seen) {
            if (!parse_u32(argv[++i], &options->seed) || options->seed == 0u) {
                fprintf(stderr, "laxity-sim: --seed must be between 1 and %u\n", UINT32_MAX);
                return false;
            }
            seed_seen = true;
        } else if (strcmp(arg, "--udp") == 0 && options->udp_arg == NULL) {
            options->udp_arg = argv[++i];
        } else if (strcmp(arg, "--output") == 0 && options->output_path == NULL) {
            options->output_path = argv[++i];
        } else {
            fprintf(stderr, "laxity-sim: duplicate or unknown flag: %s\n", arg);
            return false;
        }
    }

    if (options->scenario_arg == NULL) {
        fprintf(stderr, "laxity-sim: --scenario is required\n");
        return false;
    }
    if (options->udp_arg == NULL && options->output_path == NULL) {
        fprintf(stderr, "laxity-sim: choose --udp, --output, or both\n");
        return false;
    }
    return true;
}

static FILE *open_scenario(const char *arg, char *resolved, size_t cap)
{
    FILE *file = fopen(arg, "r");
    if (file != NULL) {
        if (snprintf(resolved, cap, "%s", arg) >= (int)cap) {
            fclose(file);
            return NULL;
        }
        return file;
    }

    if (strchr(arg, '/') != NULL || strstr(arg, ".lxs") != NULL ||
        snprintf(resolved, cap, "scenarios/%s.lxs", arg) >= (int)cap) {
        return NULL;
    }
    return fopen(resolved, "r");
}

static bool open_output(sink_t *sink, const char *path)
{
    int fd = open(path, O_WRONLY | O_CREAT | O_EXCL, 0644);
    if (fd < 0) {
        fprintf(stderr, "laxity-sim: cannot create %s: %s\n", path, strerror(errno));
        return false;
    }
    sink->file = fdopen(fd, "wb");
    if (sink->file == NULL) {
        fprintf(stderr, "laxity-sim: cannot open %s: %s\n", path, strerror(errno));
        close(fd);
        return false;
    }
    return true;
}

static bool open_udp(sink_t *sink, const char *endpoint)
{
    char copy[64];
    char *colon;
    uint32_t port;

    if (strlen(endpoint) >= sizeof copy) {
        fprintf(stderr, "laxity-sim: UDP endpoint is too long\n");
        return false;
    }
    memcpy(copy, endpoint, strlen(endpoint) + 1u);
    colon = strrchr(copy, ':');
    if (colon == NULL || colon == copy || colon[1] == '\0') {
        fprintf(stderr, "laxity-sim: --udp must be numeric IPV4:PORT\n");
        return false;
    }
    *colon = '\0';
    if (!parse_u32(colon + 1, &port) || port == 0u || port > UINT16_MAX) {
        fprintf(stderr, "laxity-sim: invalid UDP port: %s\n", colon + 1);
        return false;
    }

    memset(&sink->udp_addr, 0, sizeof sink->udp_addr);
    sink->udp_addr.sin_family = AF_INET;
    sink->udp_addr.sin_port = htons((uint16_t)port);
    if (inet_pton(AF_INET, copy, &sink->udp_addr.sin_addr) != 1) {
        fprintf(stderr, "laxity-sim: --udp must use a numeric IPv4 address: %s\n", copy);
        return false;
    }

    sink->socket_fd = socket(AF_INET, SOCK_DGRAM, 0);
    if (sink->socket_fd < 0) {
        fprintf(stderr, "laxity-sim: cannot open UDP socket: %s\n", strerror(errno));
        return false;
    }
    sink->has_udp = true;
    return true;
}

static bool sleep_ms(uint32_t milliseconds)
{
    struct timespec left = {
        .tv_sec = (time_t)(milliseconds / 1000u),
        .tv_nsec = (long)(milliseconds % 1000u) * 1000000L,
    };

    while (nanosleep(&left, &left) != 0) {
        if (errno != EINTR) {
            fprintf(stderr, "laxity-sim: pacing failed: %s\n", strerror(errno));
            return false;
        }
    }
    return true;
}

static uint16_t read_u16(const uint8_t *bytes)
{
    return (uint16_t)(bytes[0] | ((uint16_t)bytes[1] << 8));
}

static bool corrupt_batch(uint8_t *bytes, size_t length)
{
    size_t batch = QOS_FRAME_OVERHEAD + read_u16(bytes + 4);

    if (length < QOS_FRAME_OVERHEAD || batch + QOS_FRAME_OVERHEAD >= length ||
        bytes[batch] != QOS_TELEMETRY_MAGIC0 || bytes[batch + 1u] != QOS_TELEMETRY_MAGIC1 ||
        bytes[batch + 3u] != QOS_FRAME_BATCH || read_u16(bytes + batch + 4u) == 0u) {
        return false;
    }
    bytes[batch + QOS_FRAME_OVERHEAD] ^= 0x01u;
    return true;
}

static bool write_buffer(sink_t *sink, uint8_t *bytes, size_t length)
{
    if (sink->gap_next) {
        sink->gap_next = false;
        return !sink->has_udp || sleep_ms(sink->pace_ms);
    }
    if (sink->corrupt_next) {
        if (!corrupt_batch(bytes, length)) {
            fprintf(stderr, "laxity-sim: cannot corrupt the next batch\n");
            return false;
        }
        sink->corrupt_next = false;
    }

    if (sink->file != NULL && fwrite(bytes, 1u, length, sink->file) != length) {
        fprintf(stderr, "laxity-sim: output write failed: %s\n", strerror(errno));
        return false;
    }
    if (sink->has_udp) {
        ssize_t sent = sendto(sink->socket_fd, bytes, length, 0,
                              (const struct sockaddr *)&sink->udp_addr,
                              sizeof sink->udp_addr);
        if (sent < 0 || (size_t)sent != length) {
            fprintf(stderr, "laxity-sim: UDP send failed: %s\n", strerror(errno));
            return false;
        }
    }

    sink->bytes_written += length;
    sink->buffers_written++;
    return !sink->has_udp || sleep_ms(sink->pace_ms);
}

static bool drain_all(simulator_t *simulator)
{
    uint8_t buffer[LAXITY_SIM_DRAIN_CAP];
    size_t length;

    while ((length = qos_telemetry_drain(buffer, sizeof buffer)) > 0u) {
        if (!write_buffer(&simulator->sink, buffer, length)) {
            return false;
        }
    }
    return true;
}

static uint32_t random_u32(simulator_t *simulator)
{
    uint32_t x = simulator->rng;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    simulator->rng = x;
    return x;
}

static bool make_record(simulator_t *simulator, uint32_t placement, uint32_t aggressor,
                        uint32_t footprint, uint32_t base_exec, uint32_t jitter,
                        qos_infer_record_t *record)
{
    int64_t exec = (int64_t)base_exec;

    if (placement > UINT8_MAX || aggressor > UINT8_MAX || footprint > UINT8_MAX ||
        base_exec == 0u || jitter >= base_exec || base_exec > UINT32_MAX - jitter) {
        return false;
    }
    if (jitter > 0u) {
        uint64_t span = (uint64_t)jitter * 2u + 1u;
        exec += (int64_t)(random_u32(simulator) % span) - (int64_t)jitter;
    }

    memset(record, 0, sizeof *record);
    record->release_cyc = simulator->release_cyc;
    record->exec_cyc = (uint32_t)exec;
    record->region_id = (uint8_t)placement;
    if (aggressor != 0u) {
        record->aggressor_idx = (uint16_t)((footprint << 8) | aggressor);
        record->reserved = ++simulator->transfer_count;
    }
    if ((uint32_t)(record->release_cyc + record->exec_cyc) < record->release_cyc) {
        record->flags = QOS_FLAG_CYCCNT_WRAP;
    }

    simulator->release_cyc += simulator->period_cyc;
    simulator->attempted_records++;
    return true;
}

static bool push_one(simulator_t *simulator, uint32_t placement, uint32_t aggressor,
                     uint32_t footprint, uint32_t exec, uint32_t jitter, bool must_accept)
{
    qos_infer_record_t record;
    bool accepted;

    if (!make_record(simulator, placement, aggressor, footprint, exec, jitter, &record)) {
        return false;
    }
    accepted = qos_telemetry_push(&record);
    if (accepted) {
        simulator->accepted_records++;
    } else {
        simulator->rejected_records++;
    }
    return accepted || !must_accept;
}

static bool emit_records(simulator_t *simulator, uint32_t count, uint32_t placement,
                         uint32_t aggressor, uint32_t footprint, uint32_t exec,
                         uint32_t jitter, uint32_t batch)
{
    if (count == 0u || batch == 0u || batch > 32u) {
        return false;
    }
    for (uint32_t i = 0u; i < count; ++i) {
        if (!push_one(simulator, placement, aggressor, footprint, exec, jitter, true)) {
            return false;
        }
        if (((i + 1u) % batch) == 0u || i + 1u == count) {
            if (!drain_all(simulator)) {
                return false;
            }
        }
    }
    return true;
}

static bool overflow_ring(simulator_t *simulator, uint32_t count, uint32_t placement,
                          uint32_t aggressor, uint32_t footprint, uint32_t exec,
                          uint32_t jitter)
{
    if (count <= 32u || count > 100000u) {
        return false;
    }
    for (uint32_t i = 0u; i < count; ++i) {
        if (!push_one(simulator, placement, aggressor, footprint, exec, jitter, false)) {
            return false;
        }
    }
    return drain_all(simulator);
}

static int split_tokens(char *line, char **tokens)
{
    char *save = NULL;
    char *token = strtok_r(line, " \t", &save);
    int count = 0;

    while (token != NULL) {
        if (count == (int)LAXITY_SIM_TOKEN_CAP) {
            return -1;
        }
        tokens[count++] = token;
        token = strtok_r(NULL, " \t", &save);
    }
    return count;
}

static bool command_numbers(char **tokens, int first, int count, uint32_t *values)
{
    for (int i = 0; i < count; ++i) {
        if (!parse_u32(tokens[first + i], &values[i])) {
            return false;
        }
    }
    return true;
}

static bool valid_name(const char *name)
{
    if (name[0] == '\0') {
        return false;
    }
    for (const unsigned char *p = (const unsigned char *)name; *p != '\0'; ++p) {
        if (!((*p >= 'a' && *p <= 'z') || (*p >= '0' && *p <= '9') ||
              *p == '-' || *p == '_')) {
            return false;
        }
    }
    return true;
}

static bool run_scenario(FILE *file, const char *path, simulator_t *simulator, char *name,
                         size_t name_cap)
{
    char line[LAXITY_SIM_LINE_CAP];
    unsigned long line_number = 0u;
    unsigned int header_step = 0u;
    bool have_placements = false;
    bool have_action = false;

    while (fgets(line, sizeof line, file) != NULL) {
        char *tokens[LAXITY_SIM_TOKEN_CAP];
        char *text = line;
        size_t length;
        int token_count;
        uint32_t values[7];

        ++line_number;
        length = strlen(line);
        if (length > 0u && line[length - 1u] != '\n' && !feof(file)) {
            fprintf(stderr, "laxity-sim: %s:%lu: line exceeds %u bytes\n",
                    path, line_number, LAXITY_SIM_LINE_CAP - 1u);
            return false;
        }
        while (length > 0u && (line[length - 1u] == '\n' || line[length - 1u] == '\r')) {
            line[--length] = '\0';
        }
        while (*text == ' ' || *text == '\t') {
            ++text;
        }
        if (*text == '\0' || *text == '#') {
            continue;
        }

        token_count = split_tokens(text, tokens);
        if (token_count < 0) {
            fprintf(stderr, "laxity-sim: %s:%lu: too many fields\n", path, line_number);
            return false;
        }

        if (header_step == 0u && token_count == 2 && strcmp(tokens[0], "schema") == 0 &&
            strcmp(tokens[1], "1") == 0) {
            header_step++;
            continue;
        }
        if (header_step == 1u && token_count == 2 && strcmp(tokens[0], "name") == 0 &&
            valid_name(tokens[1]) && snprintf(name, name_cap, "%s", tokens[1]) < (int)name_cap) {
            header_step++;
            continue;
        }
        if (header_step == 2u && token_count == 3 && strcmp(tokens[0], "clock") == 0 &&
            command_numbers(tokens, 1, 2, values) && values[0] > 0u && values[1] > 0u) {
            simulator->clock_hz = values[0];
            simulator->cyccnt_hz = values[1];
            header_step++;
            continue;
        }
        if (header_step == 3u && token_count == 2 &&
            strcmp(tokens[0], "period_cycles") == 0 && parse_u32(tokens[1], &values[0]) &&
            values[0] > 0u) {
            simulator->period_cyc = values[0];
            header_step++;
            continue;
        }
        if (header_step == 4u && token_count == 2 && strcmp(tokens[0], "pace_ms") == 0 &&
            parse_u32(tokens[1], &simulator->sink.pace_ms) && simulator->sink.pace_ms <= 600000u) {
            header_step++;
            continue;
        }
        if (header_step == 5u && token_count == 2 && strcmp(tokens[0], "header_flags") == 0 &&
            parse_u32(tokens[1], &values[0]) && values[0] <= 3u) {
            qos_telemetry_init(simulator->clock_hz, simulator->cyccnt_hz,
                               (uint8_t)(values[0] | LAXITY_SIM_ORIGIN_FLAG));
            header_step++;
            continue;
        }
        if (header_step == 6u && token_count == 6 && strcmp(tokens[0], "null_probe") == 0 &&
            command_numbers(tokens, 1, 5, values) && values[4] <= UINT16_MAX) {
            qos_telemetry_set_null_probe(values[0], values[1], values[2], values[3],
                                         (uint16_t)values[4]);
            header_step++;
            continue;
        }
        if (header_step < 7u) {
            fprintf(stderr, "laxity-sim: %s:%lu: invalid or out-of-order header field\n",
                    path, line_number);
            return false;
        }

        if (strcmp(tokens[0], "placements") == 0 && token_count == 2) {
            uint8_t count = 0u;
            const qos_placement_t *table = laxity_sim_placements(tokens[1], &count);
            if (table == NULL) {
                fprintf(stderr, "laxity-sim: %s:%lu: unknown placement set: %s\n",
                        path, line_number, tokens[1]);
                return false;
            }
            qos_telemetry_set_placements(table, count);
            have_placements = true;
            continue;
        }
        if (!have_placements) {
            fprintf(stderr, "laxity-sim: %s:%lu: select placements before actions\n",
                    path, line_number);
            return false;
        }
        if (strcmp(tokens[0], "emit") == 0 && token_count == 8 &&
            command_numbers(tokens, 1, 7, values) &&
            emit_records(simulator, values[0], values[1], values[2], values[3], values[4],
                         values[5], values[6])) {
            have_action = true;
            continue;
        }
        if (strcmp(tokens[0], "overflow") == 0 && token_count == 7 &&
            command_numbers(tokens, 1, 6, values) &&
            overflow_ring(simulator, values[0], values[1], values[2], values[3], values[4],
                          values[5])) {
            have_action = true;
            continue;
        }
        if (strcmp(tokens[0], "set_release") == 0 && token_count == 2 &&
            parse_u32(tokens[1], &simulator->release_cyc)) {
            continue;
        }
        if (strcmp(tokens[0], "crc_next") == 0 && token_count == 1 &&
            !simulator->sink.corrupt_next && !simulator->sink.gap_next) {
            simulator->sink.corrupt_next = true;
            continue;
        }
        if (strcmp(tokens[0], "gap_next") == 0 && token_count == 1 &&
            !simulator->sink.corrupt_next && !simulator->sink.gap_next) {
            simulator->sink.gap_next = true;
            continue;
        }
        if (strcmp(tokens[0], "silence") == 0 && token_count == 2 &&
            parse_u32(tokens[1], &values[0]) && values[0] <= 600000u &&
            (!simulator->sink.has_udp || sleep_ms(values[0]))) {
            have_action = true;
            continue;
        }

        fprintf(stderr, "laxity-sim: %s:%lu: invalid command or value\n", path, line_number);
        return false;
    }

    if (ferror(file)) {
        fprintf(stderr, "laxity-sim: cannot read %s: %s\n", path, strerror(errno));
        return false;
    }
    if (header_step != 7u || !have_placements || !have_action) {
        fprintf(stderr, "laxity-sim: %s: incomplete scenario\n", path);
        return false;
    }
    if (simulator->sink.corrupt_next || simulator->sink.gap_next) {
        fprintf(stderr, "laxity-sim: %s: fault command has no following batch\n", path);
        return false;
    }
    return true;
}

static bool close_sink(sink_t *sink)
{
    bool ok = true;

    if (sink->file != NULL) {
        if (fflush(sink->file) != 0) {
            fprintf(stderr, "laxity-sim: output flush failed: %s\n", strerror(errno));
            ok = false;
        }
        if (fclose(sink->file) != 0) {
            fprintf(stderr, "laxity-sim: output close failed: %s\n", strerror(errno));
            ok = false;
        }
        sink->file = NULL;
    }
    if (sink->socket_fd >= 0) {
        if (close(sink->socket_fd) != 0) {
            fprintf(stderr, "laxity-sim: socket close failed: %s\n", strerror(errno));
            ok = false;
        }
        sink->socket_fd = -1;
    }
    return ok;
}

int main(int argc, char **argv)
{
    options_t options;
    simulator_t simulator = { .sink.socket_fd = -1 };
    char scenario_path[512];
    char scenario_name[128];
    FILE *scenario;
    bool ok;

    if (!parse_options(argc, argv, &options)) {
        usage(stderr);
        return 2;
    }
    simulator.rng = options.seed;

    scenario = open_scenario(options.scenario_arg, scenario_path, sizeof scenario_path);
    if (scenario == NULL) {
        fprintf(stderr, "laxity-sim: cannot open scenario %s: %s\n",
                options.scenario_arg, strerror(errno));
        return 1;
    }
    if (options.output_path != NULL && !open_output(&simulator.sink, options.output_path)) {
        fclose(scenario);
        return 1;
    }
    if (options.udp_arg != NULL && !open_udp(&simulator.sink, options.udp_arg)) {
        fclose(scenario);
        close_sink(&simulator.sink);
        return 1;
    }

    ok = run_scenario(scenario, scenario_path, &simulator, scenario_name, sizeof scenario_name);
    if (fclose(scenario) != 0) {
        fprintf(stderr, "laxity-sim: scenario close failed: %s\n", strerror(errno));
        ok = false;
    }
    if (!close_sink(&simulator.sink)) {
        ok = false;
    }
    if (!ok) {
        return 1;
    }

    fprintf(stderr,
            "laxity-sim: scenario=%s seed=%u attempted=%llu accepted=%llu rejected=%llu buffers=%llu bytes=%llu\n",
            scenario_name, options.seed,
            (unsigned long long)simulator.attempted_records,
            (unsigned long long)simulator.accepted_records,
            (unsigned long long)simulator.rejected_records,
            (unsigned long long)simulator.sink.buffers_written,
            (unsigned long long)simulator.sink.bytes_written);
    return 0;
}
