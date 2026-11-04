/* diag only: the console ioctls SDL_evdev_kbd.c makes, on a VT.
 * Usage: vtprobe TTY [VTNUM]   (with VTNUM: VT_PROCESS switching away and back) */
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/wait.h>
#include <unistd.h>
#include <linux/kd.h>
#include <linux/keyboard.h>
#include <linux/tiocl.h>
#include <linux/vt.h>

static volatile sig_atomic_t got_rel, got_acq;
static void on_rel(int s) { (void)s; got_rel = 1; }
static void on_acq(int s) { (void)s; got_acq = 1; }

static int wait_flag(volatile sig_atomic_t *flag, int ms)
{
    for (int i = 0; i < ms / 10 && !*flag; i++) usleep(10000);
    return *flag;
}

static void activate_in_child(int vt)
{
    pid_t pid = fork();
    if (pid == 0) {
        int fd = open("/dev/tty0", O_RDWR);
        int a = ioctl(fd, VT_ACTIVATE, vt);
        alarm(10);
        int w = ioctl(fd, VT_WAITACTIVE, vt);
        printf("  child: VT_ACTIVATE %d -> %d, VT_WAITACTIVE -> %d %s\n", vt, a, w, w ? strerror(errno) : "");
        fflush(stdout);
        _exit(0);
    }
}

int main(int argc, char **argv)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    int fd = open(argv[1], O_RDONLY | O_CLOEXEC);
    printf("open(%s, O_RDONLY) = %d %s\n", argv[1], fd, fd < 0 ? strerror(errno) : "");
    if (fd < 0) return 1;
    char kbtype = 0;
    int r0 = ioctl(fd, KDGKBTYPE, &kbtype);
    printf("KDGKBTYPE -> %d type %d (KB_101=%d)\n", r0, kbtype, KB_101);
    int mode = -1;
    r0 = ioctl(fd, KDGKBMODE, &mode);
    printf("KDGKBMODE -> %d mode %d\n", r0, mode);
    char shift[sizeof(long)] = { TIOCL_GETSHIFTSTATE, 0 };
    r0 = ioctl(fd, TIOCLINUX, shift);
    printf("TIOCLINUX(GETSHIFTSTATE) -> %d state %d\n", r0, shift[0]);
    char led = 0;
    r0 = ioctl(fd, KDGKBLED, &led);
    printf("KDGKBLED -> %d flags 0x%x\n", r0, led);
    for (int t = 0; t < 2; t++) {
        struct kbentry e = { .kb_table = t, .kb_index = 30 };
        int r = ioctl(fd, KDGKBENT, &e);
        printf("KDGKBENT(table %d, key 30) -> %d value 0x%04x\n", t, r, e.kb_value);
    }
    static struct kbdiacrs acc;
    r0 = ioctl(fd, KDGKBDIACR, &acc);
    printf("KDGKBDIACR -> %d count %u first %02x %02x %02x\n", r0, acc.kb_cnt, acc.kbdiacr[0].diacr, acc.kbdiacr[0].base, acc.kbdiacr[0].result);
    struct vt_stat st = { 0 };
    r0 = ioctl(fd, VT_GETSTATE, &st);
    printf("VT_GETSTATE -> %d active %d\n", r0, st.v_active);
    struct vt_mode vm = { 0 };
    r0 = ioctl(fd, VT_GETMODE, &vm);
    printf("VT_GETMODE -> %d mode %d\n", r0, vm.mode);
    int r = ioctl(fd, KDSKBMODE, K_UNICODE);
    printf("KDSKBMODE(K_UNICODE) -> %d %s\n", r, r ? strerror(errno) : "");
    r = ioctl(fd, KDSKBMODE, K_OFF);
    int now = -1;
    ioctl(fd, KDGKBMODE, &now);
    printf("KDSKBMODE(K_OFF) -> %d %s, now %d\n", r, r ? strerror(errno) : "", now);
    r = ioctl(fd, KDSKBMODE, mode);
    ioctl(fd, KDGKBMODE, &now);
    printf("KDSKBMODE(old %d) -> %d, now %d\n", mode, r, now);

    if (argc > 2) {
        int vt = atoi(argv[2]);
        int other = st.v_active != vt ? st.v_active : (vt == 1 ? 2 : 1);
        signal(SIGUSR1, on_rel);
        signal(SIGUSR2, on_acq);
        int t0 = open("/dev/tty0", O_RDWR);
        r = ioctl(t0, VT_ACTIVATE, vt);
        alarm(10);
        int w = ioctl(t0, VT_WAITACTIVE, vt);
        alarm(0);
        printf("VT_ACTIVATE %d -> %d, VT_WAITACTIVE -> %d\n", vt, r, w);
        struct vt_mode pm = { .mode = VT_PROCESS, .relsig = SIGUSR1, .acqsig = SIGUSR2, .frsig = SIGIO };
        r = ioctl(fd, VT_SETMODE, &pm);
        printf("VT_SETMODE(VT_PROCESS) -> %d %s\n", r, r ? strerror(errno) : "");
        activate_in_child(other);
        printf("release signal: %d\n", wait_flag(&got_rel, 5000));
        printf("VT_RELDISP(1) -> %d\n", ioctl(fd, VT_RELDISP, 1));
        wait(NULL);
        ioctl(t0, VT_GETSTATE, &st);
        printf("active now %d\n", st.v_active);
        activate_in_child(vt);
        printf("acquire signal: %d\n", wait_flag(&got_acq, 5000));
        printf("VT_RELDISP(VT_ACKACQ) -> %d\n", ioctl(fd, VT_RELDISP, VT_ACKACQ));
        wait(NULL);
        ioctl(t0, VT_GETSTATE, &st);
        printf("active now %d\n", st.v_active);
        struct vt_mode am = { .mode = VT_AUTO };
        printf("VT_SETMODE(VT_AUTO) -> %d\n", ioctl(fd, VT_SETMODE, &am));
        ioctl(t0, VT_ACTIVATE, other);
    }
    return 0;
}
