/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Stub `linux/errno.h` for the AREST linuxkpi shim. The values mirror
 * Linux's standard errno.h definitions on x86_64.
 */
#ifndef _LINUX_ERRNO_H
#define _LINUX_ERRNO_H

#define EPERM        1
#define ENOENT       2
#define EIO          5
#define ENOMEM      12
#define EFAULT      14
#define EBUSY       16
#define EEXIST      17
#define ENODEV      19
#define EINVAL      22
#define ENOSPC      28
#define EOPNOTSUPP  95

#endif /* _LINUX_ERRNO_H */
