/* libc functions Rust's std and crates name that no PS5 module provides. */
#include <stddef.h>
#include <stdint.h>

extern void arc4random_buf(void *buf, size_t nbytes);

int getrandom(void *buf, size_t buflen, unsigned int flags)
{
    (void)flags;
    arc4random_buf(buf, buflen);
    return (int)buflen;
}

int pthread_setname_np(void *thread, const char *name)
{
    (void)thread;
    (void)name;
    return 0;
}

int dl_iterate_phdr(void *callback, void *data)
{
    (void)callback;
    (void)data;
    return 0;
}

int bcmp(const void *a, const void *b, size_t n)
{
    const unsigned char *p = a, *q = b;
    for (size_t i = 0; i < n; i++)
        if (p[i] != q[i])
            return 1;
    return 0;
}

extern int pipe(int fds[2]);
extern int fcntl(int fd, int cmd, ...);
extern int *__error(void);

#define PS5_F_SETFD 2
#define PS5_F_GETFL 3
#define PS5_F_SETFL 4
#define PS5_FD_CLOEXEC 1
#define PS5_O_NONBLOCK 0x0004
#define PS5_O_CLOEXEC 0x00100000
#define PS5_ENOSYS 78

int pipe2(int fds[2], int flags)
{
    if (pipe(fds) != 0)
        return -1;
    for (int i = 0; i < 2; i++) {
        if (flags & PS5_O_CLOEXEC)
            fcntl(fds[i], PS5_F_SETFD, PS5_FD_CLOEXEC);
        if (flags & PS5_O_NONBLOCK)
            fcntl(fds[i], PS5_F_SETFL, fcntl(fds[i], PS5_F_GETFL) | PS5_O_NONBLOCK);
    }
    return 0;
}

static int unsupported(void)
{
    *__error() = PS5_ENOSYS;
    return -1;
}

int accept4(int s, void *addr, void *len, int flags) { (void)s; (void)addr; (void)len; (void)flags; return unsupported(); }
int getpeereid(int s, void *uid, void *gid) { (void)s; (void)uid; (void)gid; return unsupported(); }
int killpg(int pgrp, int sig) { (void)pgrp; (void)sig; return unsupported(); }
int mkfifo(const char *path, unsigned mode) { (void)path; (void)mode; return unsupported(); }
int setgid(unsigned gid) { (void)gid; return unsupported(); }

/* Imported by std but left NULL on the console (XPSemu's import report). */
int fork(void) { return unsupported(); }
int chroot(const char *path) { (void)path; return unsupported(); }
int setsid(void) { return unsupported(); }
int symlink(const char *target, const char *path) { (void)target; (void)path; return unsupported(); }
long readlink(const char *path, char *buf, size_t size) { (void)path; (void)buf; (void)size; return unsupported(); }
int mkstemp(char *template_) { (void)template_; return unsupported(); }
const char *gai_strerror(int code) { (void)code; return "name resolution unsupported"; }
/* Console run 2's import report. */
int setpgid(int pid, int pgid) { (void)pid; (void)pgid; return unsupported(); }
int linkat(int fd1, const char *p1, int fd2, const char *p2, int flag) { (void)fd1; (void)p1; (void)fd2; (void)p2; (void)flag; return unsupported(); }
int chown(const char *path, unsigned uid, unsigned gid) { (void)path; (void)uid; (void)gid; return unsupported(); }
int lchown(const char *path, unsigned uid, unsigned gid) { (void)path; (void)uid; (void)gid; return unsupported(); }
int fchown(int fd, unsigned uid, unsigned gid) { (void)fd; (void)uid; (void)gid; return unsupported(); }

/*
 * std built for FreeBSD 11 (the PS5's ABI) names stat@FBSD_1.0 and friends;
 * ps5-link.sh binds those names here. The SDK's headers are FreeBSD 11's, so
 * these calls take the same structures std passes.
 */
struct stat;
struct dirent;
typedef struct _dirdesc DIR;
extern int stat(const char *path, struct stat *sb);
extern int lstat(const char *path, struct stat *sb);
extern int fstat(int fd, struct stat *sb);
extern int fstatat(int fd, const char *path, struct stat *sb, int flag);
extern struct dirent *readdir(DIR *dir);

int ruffle_stat11(const char *path, struct stat *sb) { return stat(path, sb); }
int ruffle_lstat11(const char *path, struct stat *sb) { return lstat(path, sb); }
int ruffle_fstat11(int fd, struct stat *sb) { return fstat(fd, sb); }
int ruffle_fstatat11(int fd, const char *path, struct stat *sb, int flag) { return fstatat(fd, path, sb, flag); }
struct dirent *ruffle_readdir11(DIR *dir) { return readdir(dir); }

typedef void (*PFN_vkVoidFunction)(void);
extern PFN_vkVoidFunction vk_icdGetInstanceProcAddr(void *instance, const char *name);

PFN_vkVoidFunction vkGetInstanceProcAddr(void *instance, const char *name)
{
    return vk_icdGetInstanceProcAddr(instance, name);
}
