#include <X11/Xlib.h>
#include <X11/Xatom.h>
#include <X11/Xutil.h>
#include <X11/keysym.h>
#include <X11/extensions/XTest.h>
#include <errno.h>
#include <limits.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <unistd.h>

static volatile sig_atomic_t grab_failed = 0;

static int handle_x_error(Display *display, XErrorEvent *event) {
    (void)display;
    if (event->error_code == BadAccess) {
        grab_failed = 1;
    }
    return 0;
}

static void toggle_recording(void) {
    pid_t pid = fork();
    if (pid == 0) {
        const char *user_home = getenv("HOME");
        char binary[PATH_MAX];
        if (user_home == NULL) _exit(127);
        int length = snprintf(binary, sizeof(binary), "%s/.local/bin/opentyless-rs", user_home);
        if (length < 0 || (size_t)length >= sizeof(binary)) _exit(127);
        execl(binary, "opentyless-rs", "toggle-record", (char *)NULL);
        _exit(127);
    }
}

static int terminal_class(const char *name) {
    static const char *classes[] = {
        "tilix", "gnome-terminal", "gnome-terminal-server", "org.gnome.Terminal",
        "kgx", "org.gnome.Console", "konsole", "xfce4-terminal", "mate-terminal",
        "xterm", "uxterm", "urxvt", "rxvt", "kitty", "Alacritty", "wezterm",
        "org.wezfurlong.wezterm", "terminator", "terminology", "sakura",
        "st", "guake", "yakuake", "tilda", "foot", "ghostty", "com.mitchellh.ghostty"
    };
    if (name == NULL) return 0;
    for (size_t i = 0; i < sizeof(classes) / sizeof(classes[0]); ++i) {
        if (strcasecmp(name, classes[i]) == 0) return 1;
    }
    return 0;
}

static int window_is_terminal(Display *display, Window window) {
    XClassHint hint = {0};
    if (!XGetClassHint(display, window, &hint)) return 0;
    int result = terminal_class(hint.res_name) || terminal_class(hint.res_class);
    if (hint.res_name) XFree(hint.res_name);
    if (hint.res_class) XFree(hint.res_class);
    return result;
}

static int focused_terminal(Display *display) {
    Atom active = XInternAtom(display, "_NET_ACTIVE_WINDOW", True);
    Atom type;
    int format;
    unsigned long count, remaining;
    unsigned char *data = NULL;
    if (active != None &&
        XGetWindowProperty(display, DefaultRootWindow(display), active, 0, 1,
                           False, XA_WINDOW, &type, &format, &count,
                           &remaining, &data) == Success) {
        Window window = None;
        if (type == XA_WINDOW && format == 32 && count == 1 && data)
            window = *(Window *)data;
        if (data) XFree(data);
        if (window != None) return window_is_terminal(display, window);
    }
    Window window;
    int revert;
    XGetInputFocus(display, &window, &revert);
    for (int depth = 0; depth < 32 && window != None && window != PointerRoot; ++depth) {
        if (window_is_terminal(display, window)) return 1;
        Window root, parent, *children = NULL;
        unsigned int child_count;
        if (!XQueryTree(display, window, &root, &parent, &children, &child_count)) break;
        if (children) XFree(children);
        if (window == root) break;
        window = parent;
    }
    return 0;
}

static void paste_clipboard(Display *display) {
    int terminal = focused_terminal(display);
    KeyCode control = XKeysymToKeycode(display, XK_Control_L);
    KeyCode shift = XKeysymToKeycode(display, XK_Shift_L);
    KeyCode v = XKeysymToKeycode(display, XK_v);

    if (control == 0 || v == 0 || (terminal && shift == 0)) {
        fprintf(stderr, "无法找到 Ctrl 或 V 的 X11 键码\n");
        return;
    }

    XTestFakeKeyEvent(display, control, True, CurrentTime);
    if (terminal) XTestFakeKeyEvent(display, shift, True, CurrentTime);
    XTestFakeKeyEvent(display, v, True, CurrentTime);
    XTestFakeKeyEvent(display, v, False, CurrentTime);
    if (terminal) XTestFakeKeyEvent(display, shift, False, CurrentTime);
    XTestFakeKeyEvent(display, control, False, CurrentTime);
    XFlush(display);
}

int main(int argc, char **argv) {
    Display *display = XOpenDisplay(NULL);
    if (display == NULL) {
        fprintf(stderr, "无法连接 X11 显示；请检查 DISPLAY 和 XAUTHORITY\n");
        return 1;
    }

    if (argc == 2 && strcmp(argv[1], "--print-paste-shortcut") == 0) {
        puts(focused_terminal(display) ? "Ctrl+Shift+V" : "Ctrl+V");
        XCloseDisplay(display);
        return 0;
    }

    signal(SIGCHLD, SIG_IGN);
    XSetErrorHandler(handle_x_error);

    Window root = DefaultRootWindow(display);
    unsigned int event_mask = ButtonPressMask | ButtonReleaseMask;

    XGrabButton(display, 8, AnyModifier, root, False, event_mask,
                GrabModeAsync, GrabModeAsync, None, None);
    XGrabButton(display, 9, AnyModifier, root, False, event_mask,
                GrabModeAsync, GrabModeAsync, None, None);
    XSync(display, False);

    if (grab_failed) {
        fprintf(stderr, "侧键 8/9 已被其他程序占用，无法绑定\n");
        XCloseDisplay(display);
        return 2;
    }

    puts("已绑定侧键 8：终端 Ctrl+Shift+V，其他窗口 Ctrl+V；侧键 9：切换录音");
    fflush(stdout);

    for (;;) {
        XEvent event;
        XNextEvent(display, &event);
        if (event.type != ButtonPress) {
            continue;
        }

        if (event.xbutton.button == 8) {
            paste_clipboard(display);
        } else if (event.xbutton.button == 9) {
            toggle_recording();
        }
    }
}
