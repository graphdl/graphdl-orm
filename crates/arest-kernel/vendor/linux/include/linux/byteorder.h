/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Stub byte-order helpers for the AREST linuxkpi shim. AREST runs on
 * x86_64 (little-endian), so cpu_to_le* and le*_to_cpu are identity.
 * Real Linux's `linux/byteorder/generic.h` does the same on LE hosts.
 */
#ifndef _LINUX_BYTEORDER_H
#define _LINUX_BYTEORDER_H

#include <linux/types.h>

#define cpu_to_le16(x) ((__le16)(x))
#define cpu_to_le32(x) ((__le32)(x))
#define cpu_to_le64(x) ((__le64)(x))
#define le16_to_cpu(x) ((u16)(x))
#define le32_to_cpu(x) ((u32)(x))
#define le64_to_cpu(x) ((u64)(x))

#define cpu_to_be16(x) ((__be16)__builtin_bswap16(x))
#define cpu_to_be32(x) ((__be32)__builtin_bswap32(x))
#define be16_to_cpu(x) ((u16)__builtin_bswap16(x))
#define be32_to_cpu(x) ((u32)__builtin_bswap32(x))

#endif /* _LINUX_BYTEORDER_H */
