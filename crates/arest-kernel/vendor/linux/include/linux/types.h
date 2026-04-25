/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Stub `linux/types.h` for the AREST linuxkpi shim (#460 Track AAAA).
 *
 * Linux's real types.h pulls in `<asm/types.h>` plus a sprawl of
 * compat shims. We declare only the typedefs the vendored
 * `drivers/virtio/virtio_input.c` reaches transitively.
 *
 * Width assumptions:
 *   x86_64 LP64 (long = 64 bits, pointer = 64 bits).
 *
 * The Linux kernel uses both u8/u16/u32/u64 (kernel-internal) AND
 * __u8/__u16/__u32/__u64 (UAPI-facing). We declare both as the same
 * underlying types so the .c file's mixed usage compiles.
 */
#ifndef _LINUX_TYPES_H
#define _LINUX_TYPES_H

/* Use stdint.h's well-defined integer widths. Freestanding clang
 * provides this even under -ffreestanding.
 */
#include <stdint.h>
#include <stddef.h>

typedef uint8_t  u8;
typedef uint16_t u16;
typedef uint32_t u32;
typedef uint64_t u64;
typedef int8_t   s8;
typedef int16_t  s16;
typedef int32_t  s32;
typedef int64_t  s64;

typedef uint8_t  __u8;
typedef uint16_t __u16;
typedef uint32_t __u32;
typedef uint64_t __u64;
typedef int8_t   __s8;
typedef int16_t  __s16;
typedef int32_t  __s32;
typedef int64_t  __s64;

/* Linux's little-endian-tagged types — purely for source readability;
 * we don't enforce endianness at the type system level. virtio_input.c
 * uses cpu_to_le16 / le16_to_cpu macros which on x86_64 are no-ops.
 */
typedef u16 __le16;
typedef u32 __le32;
typedef u64 __le64;
typedef u16 __be16;
typedef u32 __be32;
typedef u64 __be64;

typedef int bool;
#define true  1
#define false 0

/* Kernel-style "unsigned long" — used widely in bitmaps. Word-width
 * (8 bytes on x86_64). */
typedef unsigned long ulong;

/* Sentinel used by Linux's gfp / kmalloc family. We accept any int
 * value; see linuxkpi alloc.rs.
 */
typedef unsigned int gfp_t;

#endif /* _LINUX_TYPES_H */
