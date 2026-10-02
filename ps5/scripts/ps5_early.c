/*
 * Ruffle Flash PS5 start-up, run from .preinit_array before any constructor or
 * Rust code (ported from XPSemu's ui/xemu-os-utils-ps5.c): HEN jailbreak
 * request, log to /data/ruffle/ruffle.log, unresolved-import report and a
 * crash handler that toasts the eboot offset.
 */
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <pthread.h>
#include <time.h>
#include <unistd.h>

#define RUFFLE_DIR "/data/ruffle"
#define RUFFLE_LOG RUFFLE_DIR "/ruffle.log"
#define TITLE_ID "PPSA68091"

#define EBOOT_BASE 0x400000ULL
#define EBOOT_END 0x10000000ULL

int sceKernelUsleep(unsigned int microseconds);
int sceKernelSendNotificationRequest(int device, void *request, size_t size, int blocking);

static void notify(const char *message)
{
    static uint8_t request[0xc30];
    memset(request, 0, sizeof(request));
    snprintf((char *)request + 0x2d, 1024, "%s", message);
    sceKernelSendNotificationRequest(0, request, sizeof(request), 0);
}

static const char *volatile stage = "start-up";
static int log_fd = -1;

/* Whole lines straight to the log file's descriptor: stdio and fds 1-2 may
 * not reach it on the PS5 (Rust's println and panics didn't). */
void ruffle_ps5_log(const char *text)
{
    if (log_fd < 0)
        return;
    size_t n = strlen(text);
    (void)!write(log_fd, text, n);
    if (n == 0 || text[n - 1] != '\n')
        (void)!write(log_fd, "\n", 1);
}

static void logf_(const char *fmt, ...) __attribute__((format(printf, 1, 2)));
static void logf_(const char *fmt, ...)
{
    char buf[1024];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(buf, sizeof(buf), fmt, ap);
    va_end(ap);
    ruffle_ps5_log(buf);
}

/*
 * USB keyboard. libSceKeyboard isn't loaded for a title, so linking against it
 * left empty imports (and the startup "unresolved imports" toast); the module
 * is loaded here at run time instead and its functions looked up.
 */
int sceKernelLoadStartModule(const char *path, size_t argc, const void *argv, uint32_t flags,
                             void *opt, int *result);
int sceKernelDlsym(int handle, const char *symbol, void **address);

static int (*kb_init)(void);
static int (*kb_open)(int user, int type, int index, const void *param);
static int (*kb_read_state)(int handle, void *data);

/* 0 when the keyboard functions are ready. */
int ruffle_ps5_keyboard_load(void)
{
    static const char *const paths[] = {
        "/system/common/lib/libSceKeyboard.sprx",
        "/system/priv/lib/libSceKeyboard.sprx",
        "libSceKeyboard.sprx",
    };
    int handle = -1;
    for (size_t i = 0; i < sizeof(paths) / sizeof(paths[0]) && handle < 0; i++) {
        handle = sceKernelLoadStartModule(paths[i], 0, NULL, 0, NULL, NULL);
        logf_("Ruffle PS5: keyboard: load %s -> %#x", paths[i], handle);
    }
    if (handle < 0)
        return -1;
    int rc = sceKernelDlsym(handle, "sceKeyboardInit", (void **)&kb_init);
    rc |= sceKernelDlsym(handle, "sceKeyboardOpen", (void **)&kb_open);
    rc |= sceKernelDlsym(handle, "sceKeyboardReadState", (void **)&kb_read_state);
    logf_("Ruffle PS5: keyboard: symbols %s", rc == 0 && kb_init && kb_open && kb_read_state ? "found" : "missing");
    return (rc == 0 && kb_init && kb_open && kb_read_state) ? 0 : -1;
}

int ruffle_ps5_keyboard_init(void) { return kb_init ? kb_init() : -1; }
int ruffle_ps5_keyboard_open(int user, int type, int index, const void *param)
{
    return kb_open ? kb_open(user, type, index, param) : -1;
}
int ruffle_ps5_keyboard_read_state(int handle, void *data)
{
    return kb_read_state ? kb_read_state(handle, data) : -1;
}

/* Local wall-clock time for the library's clock; 0 on success. */
int ruffle_ps5_clock(int *hour, int *minute)
{
    time_t now = time(NULL);
    struct tm tm;
    if (now == (time_t)-1 || !localtime_r(&now, &tm))
        return -1;
    *hour = tm.tm_hour;
    *minute = tm.tm_min;
    return 0;
}

void ruffle_ps5_notify(const char *message)
{
    notify(message);
}

void ruffle_ps5_stage(const char *name)
{
    stage = name;
    logf_("Ruffle PS5: stage: %s", name);
}

#define HEN_REQUEST "/download0/etahen_jailbreak"
#define HEN_REQUEST_STAGED "/download0/etahen_jailbreak.tmp"
#define HEN_POLL_US 16667
#define HEN_MAX_POLLS 600
#define HEN_POST_CONSUME_POLLS 450

static bool hen_jailbreak(char *msg, size_t msg_size)
{
    int pid = getpid();
    int uid_before = geteuid();
    if (uid_before == 0) {
        snprintf(msg, msg_size, "already root (pid %d)", pid);
        return true;
    }

    char request[32];
    int len = snprintf(request, sizeof(request), "{\"PID\":%d}\n", pid);

    if ((unlink(HEN_REQUEST) != 0 && errno != ENOENT) ||
        (unlink(HEN_REQUEST_STAGED) != 0 && errno != ENOENT)) {
        snprintf(msg, msg_size, "stale request cleanup failed, errno %d", errno);
        return false;
    }
    int fd = open(HEN_REQUEST_STAGED, O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, 0666);
    if (fd < 0) {
        snprintf(msg, msg_size, "request open failed, errno %d", errno);
        return false;
    }
    bool ok = fchmod(fd, 0666) == 0 && write(fd, request, len) == len && fsync(fd) == 0;
    ok = (close(fd) == 0) && ok;
    if (!ok || rename(HEN_REQUEST_STAGED, HEN_REQUEST) != 0) {
        snprintf(msg, msg_size, "request write failed, errno %d", errno);
        unlink(HEN_REQUEST_STAGED);
        return false;
    }

    int polls = 0;
    while (access(HEN_REQUEST, F_OK) == 0) {
        if (++polls >= HEN_MAX_POLLS) {
            unlink(HEN_REQUEST);
            snprintf(msg, msg_size,
                     "no HEN took the request (pid %d): load the Helper and add "
                     TITLE_ID " to /data/whitelist.txt", pid);
            return false;
        }
        sceKernelUsleep(HEN_POLL_US);
    }
    int uid = geteuid();
    for (int i = 0; i < HEN_POST_CONSUME_POLLS && uid != 0; i++) {
        sceKernelUsleep(HEN_POLL_US);
        uid = geteuid();
    }
    snprintf(msg, msg_size, "request taken after %d polls, uid %d -> %d", polls, uid_before, uid);
    return uid == 0;
}

static void write_str(const char *s)
{
    (void)!write(log_fd >= 0 ? log_fd : STDERR_FILENO, s, strlen(s));
}

static void crash_handler(int sig, siginfo_t *info, void *context)
{
    static volatile sig_atomic_t in_handler;
    char buf[200];

    if (in_handler)
        _exit(128 + sig);
    in_handler = 1;

    /* PS5 ucontext: rbp +0x88, rip +0xe0, rsp +0xf8 (PS5SX2 ProsperoCrash.cpp). */
    const uint64_t *ctx = (const uint64_t *)context;
    uint64_t rip = ctx[0xe0 / 8], rsp = ctx[0xf8 / 8], rbp = ctx[0x88 / 8];
    snprintf(buf, sizeof(buf),
             "\n*** Ruffle crashed: signal %d, fault address %p, thread %p, stage %s\n"
             "*** rip %#lx rsp %#lx rbp %#lx (eboot+%#lx)\n",
             sig, info->si_addr, (void *)pthread_self(), stage, (unsigned long)rip,
             (unsigned long)rsp, (unsigned long)rbp, (unsigned long)(rip - EBOOT_BASE));
    write_str(buf);

    char toast[200];
    snprintf(toast, sizeof(toast), "Ruffle crashed: signal %d at eboot+%#lx (%s)",
             sig, (unsigned long)(rip - EBOOT_BASE), stage);
    notify(toast);

    if (rsp >= 0x100000 && (rsp & 7) == 0) {
        const uint64_t *sp = (const uint64_t *)rsp;
        for (int i = 0, found = 0; i < 256 && found < 24; i++) {
            if (sp[i] >= EBOOT_BASE && sp[i] < EBOOT_END) {
                snprintf(buf, sizeof(buf), "***   stack[%d] eboot+%#lx\n", i,
                         (unsigned long)(sp[i] - EBOOT_BASE));
                write_str(buf);
                found++;
            }
        }
    }
    _exit(128 + sig);
}

static void install_crash_handler(void)
{
    static uint8_t alt_stack[64 * 1024] __attribute__((aligned(16)));
    stack_t ss = { .ss_sp = alt_stack, .ss_size = sizeof(alt_stack) };
    sigaltstack(&ss, NULL);

    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_sigaction = crash_handler;
    sa.sa_flags = SA_SIGINFO | SA_ONSTACK;
    sigemptyset(&sa.sa_mask);
    const int signals[] = { SIGSEGV, SIGBUS, SIGILL, SIGFPE, SIGABRT, SIGSYS, SIGTRAP, SIGEMT };
    for (size_t i = 0; i < sizeof(signals) / sizeof(signals[0]); i++)
        sigaction(signals[i], &sa, NULL);
}

/* Unresolved import names, " name1 name2 ", for ruffle_ps5_import_missing. */
static char missing_names[4096] = " ";

/* 1 when the console left this import empty: calling it would jump to 0. */
int ruffle_ps5_import_missing(const char *name)
{
    char key[160];
    snprintf(key, sizeof(key), " %s ", name);
    return strstr(missing_names, key) != NULL;
}

static void report_unresolved_imports(void)
{
    static const char *const paths[] = {
        "/app0/imports.txt",
        "/data/homebrew/" TITLE_ID "/imports.txt",
    };
    FILE *f = NULL;
    for (size_t i = 0; !f && i < sizeof(paths) / sizeof(paths[0]); i++)
        f = fopen(paths[i], "r");
    if (!f) {
        logf_("Ruffle PS5: can't read imports.txt, errno %d", errno);
        return;
    }

    char name[128], list[512] = "";
    unsigned long offset;
    int checked = 0, missing = 0;
    while (fscanf(f, "%lx %127s", &offset, name) == 2) {
        checked++;
        if (*(const uint64_t *)(EBOOT_BASE + offset) == 0) {
            logf_("Ruffle PS5: unresolved import: %s", name);
            size_t used = strlen(missing_names);
            if (used + strlen(name) + 2 < sizeof(missing_names))
                snprintf(missing_names + used, sizeof(missing_names) - used, "%s ", name);
            if (missing++ < 12)
                snprintf(list + strlen(list), sizeof(list) - strlen(list), "%s%s",
                         missing > 1 ? " " : "", name);
        }
    }
    fclose(f);
    logf_("Ruffle PS5: %d of %d imports unresolved", missing, checked);

    if (missing) {
        char toast[600];
        snprintf(toast, sizeof(toast), "Ruffle: %d unresolved imports: %s", missing, list);
        notify(toast);
    }
}

static void ruffle_ps5_early_init(void)
{
    /* Crashes from here on get a toast, even before /data is reachable. */
    install_crash_handler();

    ruffle_ps5_stage("jailbreak");
    char jailbreak_msg[192];
    bool unlocked = hen_jailbreak(jailbreak_msg, sizeof(jailbreak_msg));

    mkdir(RUFFLE_DIR, 0777);
    mkdir(RUFFLE_DIR "/games", 0777);

    rename(RUFFLE_LOG, RUFFLE_LOG ".old");
    log_fd = open(RUFFLE_LOG, O_WRONLY | O_CREAT | O_TRUNC | O_APPEND, 0666);
    bool logging = log_fd >= 0;
    int rc_out = -2, rc_err = -2;
    if (logging) {
        /* Also on 1 and 2, for RADV's and libc's own messages. */
        rc_out = dup2(log_fd, STDOUT_FILENO);
        rc_err = dup2(log_fd, STDERR_FILENO);
    }

    if (!unlocked || !logging) {
        char toast[300];
        snprintf(toast, sizeof(toast), "Ruffle: jailbreak %s: %s%s",
                 unlocked ? "ok" : "NOT CONFIRMED", jailbreak_msg,
                 logging ? "" : " (can't write " RUFFLE_LOG ")");
        notify(toast);
    }

    logf_("Ruffle PS5: load address marker ruffle_ps5_early_init=%p",
          (void *)ruffle_ps5_early_init);
    logf_("Ruffle PS5: log fd %d, dup2 -> 1: %d, -> 2: %d (errno %d)", log_fd, rc_out, rc_err,
          errno);
    logf_("Ruffle PS5: jailbreak %s: %s", unlocked ? "ok" : "NOT CONFIRMED", jailbreak_msg);
    report_unresolved_imports();
    /* Rust code runs on a re-installed handler too (it may replace ours). */
    install_crash_handler();
    ruffle_ps5_stage("constructors");
}

__attribute__((section(".preinit_array"), used))
static void (*const ruffle_ps5_preinit)(void) = ruffle_ps5_early_init;
