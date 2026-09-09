/* see net_udp_export.c. the UART exporter keeps running whatever this does. */
#ifndef LAXITY_NET_UDP_EXPORT_H
#define LAXITY_NET_UDP_EXPORT_H

#include "nx_api.h"

/* brings the stack up and blocks until DHCP answers or the wait expires. */
UINT qos_net_start(void);

/* sends one framed buffer as a datagram, best effort. */
UINT qos_net_send(const UCHAR *data, UINT length);

/* the address DHCP handed over, and whether the link is usable. */
UINT qos_net_address(ULONG *addr, ULONG *mask);
UINT qos_net_stats(UINT *sent, UINT *failed);

#endif /* LAXITY_NET_UDP_EXPORT_H */
