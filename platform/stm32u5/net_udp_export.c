/* join a network, take an address by DHCP, and send framed telemetry batches as UDP datagrams.
 *
 * this sits beside the UART exporter rather than replacing it. the UART path is what a capture
 * falls back to when the room's network misbehaves, and the wire format is identical on both, so
 * the host parser reads either without knowing which it received.
 *
 * the credentials and the destination come from a header that is not in the repository.
 */

#include <stdint.h>

#include "nx_api.h"
#include "nxd_dhcp_client.h"
#include "nx_driver_emw3080.h"

#include "main.h"
#include "wifi_credentials.h"
#include "net_udp_export.h"

/* IP_ADDRESS counts its arguments before expanding them, so the four octets have to be expanded
 * one level early or the whole macro arrives as a single argument. */
#define LAXITY_IP4(...) IP_ADDRESS(__VA_ARGS__)

/* one payload has to hold a header frame with its placement entries and a batch, and the driver
 * prepends a physical header, so the pool payload is sized above the largest frame pair rather
 * than at the Ethernet MTU. */
#define LAXITY_NET_PAYLOAD    1568
#define LAXITY_NET_PACKETS    12
#define LAXITY_NET_POOL_BYTES ((LAXITY_NET_PAYLOAD + sizeof(NX_PACKET)) * LAXITY_NET_PACKETS)

#define LAXITY_NET_IP_STACK   2048
#define LAXITY_NET_ARP_CACHE  1024

static NX_PACKET_POOL  s_pool;
static NX_IP           s_ip;
static NX_DHCP         s_dhcp;
static NX_UDP_SOCKET   s_sock;

static UCHAR s_pool_mem[LAXITY_NET_POOL_BYTES] __attribute__((aligned(4)));
static UCHAR s_ip_stack[LAXITY_NET_IP_STACK]   __attribute__((aligned(4)));
static UCHAR s_arp_cache[LAXITY_NET_ARP_CACHE] __attribute__((aligned(4)));

static ULONG s_addr;
static ULONG s_mask;
static UINT  s_up;
static UINT  s_sent;
static UINT  s_failed;

UINT qos_net_address(ULONG *addr, ULONG *mask)
{
    if (addr != NULL) { *addr = s_addr; }
    if (mask != NULL) { *mask = s_mask; }
    return s_up;
}

UINT qos_net_stats(UINT *sent, UINT *failed)
{
    if (sent != NULL)   { *sent = s_sent; }
    if (failed != NULL) { *failed = s_failed; }
    return s_up;
}

/* brings the stack up and blocks until DHCP hands over an address.
 *
 * this runs in its own thread rather than at start up, because the driver's own threads have to
 * be running for the link to come up and nothing here may block the measurement thread. */
UINT qos_net_start(void)
{
    UINT rc;

    nx_system_initialize();

    rc = nx_packet_pool_create(&s_pool, "laxity pool", LAXITY_NET_PAYLOAD,
                               s_pool_mem, sizeof s_pool_mem);
    if (rc != NX_SUCCESS) { return rc; }

    /* address and mask are left at zero: DHCP supplies both. */
    rc = nx_ip_create(&s_ip, "laxity ip", 0u, 0u, &s_pool, nx_driver_emw3080_entry,
                      s_ip_stack, sizeof s_ip_stack, 10u);
    if (rc != NX_SUCCESS) { return rc; }

    rc = nx_arp_enable(&s_ip, s_arp_cache, sizeof s_arp_cache);
    if (rc != NX_SUCCESS) { return rc; }

    /* ICMP is not needed to send, and it is what makes the board answer a ping, which is the
     * cheapest way to tell from the host that the link is real. */
    (void)nx_icmp_enable(&s_ip);

    rc = nx_udp_enable(&s_ip);
    if (rc != NX_SUCCESS) { return rc; }

    rc = nx_dhcp_create(&s_dhcp, &s_ip, "laxity");
    if (rc != NX_SUCCESS) { return rc; }

    rc = nx_dhcp_start(&s_dhcp);
    if (rc != NX_SUCCESS) { return rc; }

    /* NX_WAIT_FOREVER would hide a failure to join as a silent hang, so this bounds the wait and
     * reports instead. thirty seconds is several DHCP retries. */
    /* the status word needs its own variable. passing the return code's address here aliases the
     * two, so the check overwrites rc with the status bits and a zero status reads as success. */
    {
        ULONG status = 0u;
        rc = nx_ip_status_check(&s_ip, NX_IP_ADDRESS_RESOLVED, &status,
                                30u * NX_IP_PERIODIC_RATE);
        if (rc != NX_SUCCESS) { return rc; }
    }
    rc = nx_ip_address_get(&s_ip, &s_addr, &s_mask);
    if (rc != NX_SUCCESS) { return rc; }
    if (s_addr == 0u) { return NX_NOT_SUCCESSFUL; }

    rc = nx_udp_socket_create(&s_ip, &s_sock, "laxity telemetry",
                              NX_IP_NORMAL, NX_FRAGMENT_OKAY, NX_IP_TIME_TO_LIVE, 8);
    if (rc != NX_SUCCESS) { return rc; }

    rc = nx_udp_socket_bind(&s_sock, NX_ANY_PORT, NX_WAIT_FOREVER);
    if (rc != NX_SUCCESS) { return rc; }

    s_up = 1u;
    return NX_SUCCESS;
}

/* send one already framed buffer.
 *
 * NX_NO_WAIT throughout: export is best effort and a full packet pool must never stall the
 * thread that is draining the telemetry ring. a dropped datagram costs records, and the parser
 * already reports what it did not receive.
 */
UINT qos_net_send(const UCHAR *data, UINT length)
{
    NX_PACKET *pkt;
    UINT rc;

    if (!s_up || data == NULL || length == 0u) { return NX_NOT_ENABLED; }

    rc = nx_packet_allocate(&s_pool, &pkt, NX_UDP_PACKET, NX_NO_WAIT);
    if (rc != NX_SUCCESS) { s_failed++; return rc; }

    rc = nx_packet_data_append(pkt, (VOID *)data, length, &s_pool, NX_NO_WAIT);
    if (rc != NX_SUCCESS) { nx_packet_release(pkt); s_failed++; return rc; }

    rc = nx_udp_socket_send(&s_sock, pkt, LAXITY_IP4(LAXITY_TELEMETRY_IP4), LAXITY_TELEMETRY_PORT);
    if (rc != NX_SUCCESS) { nx_packet_release(pkt); s_failed++; return rc; }

    s_sent++;
    return NX_SUCCESS;
}
