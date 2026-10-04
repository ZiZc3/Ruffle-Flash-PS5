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
 * USB keyboard and mouse. libSceKeyboard and libSceMouse aren't loaded for a
 * title: they come in through sysmodule IDs 0x0106 and 0x00a9 (as the
 * device-tested ps5-native-gamepad-input-research probe does; loading the
 * .sprx file directly failed with 0x80020063), and their functions are then
 * looked up, so no import is left empty when no keyboard or mouse is used.
 */
int sceKernelLoadStartModule(const char *path, size_t argc, const void *argv, uint32_t flags,
                             void *opt, int *result);
int sceKernelDlsym(int handle, const char *symbol, void **address);
int sceKernelGetModuleList(int *handles, size_t count, size_t *actual);
int sceKernelGetModuleInfo(int handle, void *info);
int sceSysmoduleLoadModule(uint16_t id);
int ruffle_ps5_import_missing(const char *name);

struct hid_api {
    const char *what;     /* "keyboard" */
    uint16_t sysmodule;   /* public sysmodule ID */
    const char *module;   /* "libSceKeyboard" */
    const char *names[4]; /* init, open, read, close */
    int (*init)(void);
    int (*open)(int user, int type, int index, const void *param);
    int (*read)(int handle, void *data, int count);
    int (*close)(int handle);
    int loaded; /* 0 not tried, 1 ready, -1 failed */
};

static struct hid_api hid_apis[2] = {
    { "keyboard", 0x0106, "libSceKeyboard",
      { "sceKeyboardInit", "sceKeyboardOpen", "sceKeyboardRead", "sceKeyboardClose" } },
    { "mouse", 0x00a9, "libSceMouse",
      { "sceMouseInit", "sceMouseOpen", "sceMouseRead", "sceMouseClose" } },
};

static int hid_preload_rc[2];

/*
 * The functions imported directly (as the device-tested probe does): the
 * loader fills them in once the sysmodule is loaded. Read through volatile
 * pointers, since an empty import reads as NULL.
 */
int sceKeyboardInit(void);
int sceKeyboardOpen(int user, int type, int index, const void *param);
int sceKeyboardRead(int handle, void *data, int count);
int sceKeyboardClose(int handle);
int sceMouseInit(void);
int sceMouseOpen(int user, int type, int index, const void *param);
int sceMouseRead(int handle, void *data, int count);
int sceMouseClose(int handle);

static void hid_imports(int which, void *fns[4])
{
    void *volatile v[4];
    if (which == 0) {
        v[0] = (void *)sceKeyboardInit;
        v[1] = (void *)sceKeyboardOpen;
        v[2] = (void *)sceKeyboardRead;
        v[3] = (void *)sceKeyboardClose;
    } else {
        v[0] = (void *)sceMouseInit;
        v[1] = (void *)sceMouseOpen;
        v[2] = (void *)sceMouseRead;
        v[3] = (void *)sceMouseClose;
    }
    for (int i = 0; i < 4; i++)
        fns[i] = v[i];
}

/* The handle of a loaded module whose name contains `module`, or -1. */
static int find_module(const char *module)
{
    static int handles[512];
    size_t count = 0;
    int rc = sceKernelGetModuleList(handles, sizeof(handles) / sizeof(handles[0]), &count);
    if (rc != 0) {
        logf_("Ruffle PS5: module list -> %#x", rc);
        return -1;
    }
    /* SceKernelModuleInfo's size differs between systems: try a few. */
    static const uint64_t sizes[] = { 0x160, 0x1a8, 0x1b0, 0x158, 0x200 };
    int first_rc = 0, named = 0;
    char some[200] = "";
    for (size_t i = 0; i < count; i++) {
        for (size_t k = 0; k < sizeof(sizes) / sizeof(sizes[0]); k++) {
            struct { uint64_t size; char name[256]; uint8_t rest[0x400]; } info;
            memset(&info, 0, sizeof(info));
            info.size = sizes[k];
            int irc = sceKernelGetModuleInfo(handles[i], &info);
            if (irc != 0) {
                if (!first_rc)
                    first_rc = irc;
                continue;
            }
            named++;
            if (strlen(some) + strlen(info.name) + 2 < sizeof(some) && i < 12) {
                strcat(some, info.name);
                strcat(some, " ");
            }
            if (strstr(info.name, module))
                return handles[i];
            break;
        }
    }
    logf_("Ruffle PS5: %s not in %zu modules (%d named, info rc %#x): %s", module, count, named,
          first_rc, some);
    return -1;
}

/* 0 when the keyboard (0) or mouse (1) functions are ready. */
int ruffle_ps5_hid_load(int which)
{
    if (which < 0 || which > 1)
        return -1;
    struct hid_api *api = &hid_apis[which];
    if (api->loaded)
        return api->loaded > 0 ? 0 : -1;
    api->loaded = -1;

    if (hid_preload_rc[which] != 0 && !ruffle_ps5_import_missing("sceSysmoduleLoadModule"))
        logf_("Ruffle PS5: %s: sysmodule %#x -> %#x", api->what, api->sysmodule,
              sceSysmoduleLoadModule(api->sysmodule));

    /* The direct imports first; else look the module up and ask it. */
    void *fns[4] = { NULL };
    hid_imports(which, fns);
    int handle = 0;
    logf_("Ruffle PS5: %s: imports %s", api->what, fns[0] && fns[1] && fns[2] ? "bound" : "empty");
    if (!(fns[0] && fns[1] && fns[2])) {
        handle = find_module(api->module);
        if (handle < 0) {
            char path[96];
            snprintf(path, sizeof(path), "/system/common/lib/%s.sprx", api->module);
            handle = sceKernelLoadStartModule(path, 0, NULL, 0, NULL, NULL);
            logf_("Ruffle PS5: %s: load %s -> %#x", api->what, path, handle);
            if (handle < 0)
                return -1;
        }
        for (int i = 0; i < 4; i++)
            sceKernelDlsym(handle, api->names[i], &fns[i]);
    }
    api->init = (int (*)(void))fns[0];
    api->open = (int (*)(int, int, int, const void *))fns[1];
    api->read = (int (*)(int, void *, int))fns[2];
    api->close = (int (*)(int))fns[3];
    int ok = api->init && api->open && api->read;
    logf_("Ruffle PS5: %s: module %#x, functions %s", api->what, handle, ok ? "found" : "missing");
    if (!ok)
        return -1;
    api->loaded = 1;
    return 0;
}

int ruffle_ps5_hid_init(int which)
{
    return which >= 0 && which <= 1 && hid_apis[which].init ? hid_apis[which].init() : -1;
}

/* Type 0 (standard) at `index`; a handle, or negative. */
int ruffle_ps5_hid_open(int which, int user, int index, const void *param)
{
    return which >= 0 && which <= 1 && hid_apis[which].open ? hid_apis[which].open(user, 0, index, param) : -1;
}

/* Records read (oldest first), 0 for none, negative on error. */
int ruffle_ps5_hid_read(int which, int handle, void *data, int count)
{
    return which >= 0 && which <= 1 && hid_apis[which].read ? hid_apis[which].read(handle, data, count) : -1;
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

/*
 * A sampling profiler (as XPSemu's): a sampler thread signals the game
 * thread 500 times a second; the handler notes where it was (rip). Samples
 * in the eboot are counted per 16-byte block; samples in system libraries
 * (malloc, memcpy...) are counted as "system", with the first eboot address
 * on the stack counted as their caller. Report: eboot offsets, read with
 * llvm-addr2line -f -C -e <the build's ELF> <offset>.
 */
#define PROF_HZ 500
#define PROF_SLOTS 8192

typedef struct {
    uint64_t addr;
    uint32_t count;
} ProfSlot;

static ProfSlot prof_self[PROF_SLOTS], prof_callers[PROF_SLOTS];
static volatile uint32_t prof_total, prof_eboot, prof_system;
static volatile bool prof_on;
static pthread_t prof_thread;
static volatile bool prof_registered;

static void prof_count(ProfSlot *table, uint64_t addr)
{
    uint64_t key = addr & ~0xfULL;
    uint32_t h = (uint32_t)((key >> 4) * 2654435761u) % PROF_SLOTS;
    for (int probe = 0; probe < 64; probe++) {
        ProfSlot *slot = &table[(h + probe) % PROF_SLOTS];
        if (slot->addr == key) {
            slot->count++;
            return;
        }
        if (slot->addr == 0) {
            slot->addr = key;
            slot->count = 1;
            return;
        }
    }
}

static void prof_handler(int sig, siginfo_t *info, void *context)
{
    (void)sig;
    (void)info;
    if (!prof_on || !prof_registered || !pthread_equal(pthread_self(), prof_thread))
        return;
    int saved_errno = errno;
    const uint64_t *ctx = (const uint64_t *)context;
    uint64_t rip = ctx[0xe0 / 8], rsp = ctx[0xf8 / 8];
    prof_total++;
    if (rip >= EBOOT_BASE && rip < EBOOT_END) {
        prof_eboot++;
        prof_count(prof_self, rip);
    } else {
        prof_system++;
        if (rsp >= 0x100000 && (rsp & 7) == 0) {
            const uint64_t *sp = (const uint64_t *)rsp;
            for (int i = 0; i < 32; i++) {
                if (sp[i] >= EBOOT_BASE && sp[i] < EBOOT_END) {
                    prof_count(prof_callers, sp[i]);
                    break;
                }
            }
        }
    }
    errno = saved_errno;
}

static void *prof_sampler(void *arg)
{
    (void)arg;
    const struct timespec period = { 0, 1000000000 / PROF_HZ };
    for (;;) {
        if (prof_on && prof_registered)
            pthread_kill(prof_thread, SIGPROF);
        nanosleep(&period, NULL);
    }
    return NULL;
}

/* Called by the game thread: it's the one sampled. */
void ruffle_ps5_profile_register(void)
{
    static bool started;
    if (!started) {
        started = true;
        struct sigaction sa;
        memset(&sa, 0, sizeof(sa));
        sa.sa_sigaction = prof_handler;
        sa.sa_flags = SA_SIGINFO | SA_RESTART;
        sigemptyset(&sa.sa_mask);
        int rc_sa = sigaction(SIGPROF, &sa, NULL) == 0 ? 0 : errno;
        pthread_t t;
        int rc_t = pthread_create(&t, NULL, prof_sampler, NULL);
        if (rc_t == 0)
            pthread_detach(t);
        logf_("Ruffle PS5: profiler: sigaction %d, sampler %d", rc_sa, rc_t);
    }
    sigset_t set;
    sigemptyset(&set);
    sigaddset(&set, SIGPROF);
    pthread_sigmask(SIG_UNBLOCK, &set, NULL);
    prof_thread = pthread_self();
    prof_registered = true;
}

void ruffle_ps5_profile_start(void)
{
    prof_on = false;
    memset(prof_self, 0, sizeof(prof_self));
    memset(prof_callers, 0, sizeof(prof_callers));
    prof_total = prof_eboot = prof_system = 0;
    prof_on = true;
}

void ruffle_ps5_profile_stop(void)
{
    prof_on = false;
}

static int prof_cmp(const void *a, const void *b)
{
    const ProfSlot *x = a, *y = b;
    return (int)y->count - (int)x->count;
}

/* Writes the report to the log: where the game thread spent its time. */
void ruffle_ps5_profile_report(const char *title)
{
    bool was_on = prof_on;
    prof_on = false;
    uint32_t total = prof_total;
    if (!total) {
        logf_("[Profile] %s: no samples", title);
        prof_on = was_on;
        return;
    }
    logf_("[Profile] %s: %u samples (%d/s), app %.1f%%, system %.1f%%", title, total, PROF_HZ,
          100.0 * prof_eboot / total, 100.0 * prof_system / total);
    static ProfSlot top[PROF_SLOTS];
    memcpy(top, prof_self, sizeof(top));
    qsort(top, PROF_SLOTS, sizeof(ProfSlot), prof_cmp);
    for (int i = 0; i < 120 && top[i].count; i++)
        logf_("[Profile]   eboot+%#lx %.2f%%", (unsigned long)(top[i].addr - EBOOT_BASE),
              100.0 * top[i].count / total);
    memcpy(top, prof_callers, sizeof(top));
    qsort(top, PROF_SLOTS, sizeof(ProfSlot), prof_cmp);
    for (int i = 0; i < 40 && top[i].count; i++)
        logf_("[Profile]   system from eboot+%#lx %.2f%%", (unsigned long)(top[i].addr - EBOOT_BASE),
              100.0 * top[i].count / total);
    prof_on = was_on;
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

/* Keyboard and mouse sysmodule results from before the jailbreak. */
static int hid_preload_rc[2] = { 1, 1 };

static void ruffle_ps5_early_init(void)
{
    /* Crashes from here on get a toast, even before /data is reachable. */
    install_crash_handler();

    /* The keyboard and mouse libraries load while the app is still a plain
     * title: after the helper's authid change the kernel refused them
     * (load_prx -> 0x63, from sysmodule and from the file alike). */
    hid_preload_rc[0] = sceSysmoduleLoadModule(hid_apis[0].sysmodule);
    hid_preload_rc[1] = sceSysmoduleLoadModule(hid_apis[1].sysmodule);

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
    logf_("Ruffle PS5: before jailbreak: keyboard sysmodule -> %#x, mouse sysmodule -> %#x",
          hid_preload_rc[0], hid_preload_rc[1]);
    /* Rust code runs on a re-installed handler too (it may replace ours). */
    install_crash_handler();
    ruffle_ps5_stage("constructors");
}

__attribute__((section(".preinit_array"), used))
static void (*const ruffle_ps5_preinit)(void) = ruffle_ps5_early_init;
