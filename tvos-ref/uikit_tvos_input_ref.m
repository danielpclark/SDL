/* Upstream reference for sdl3/tests/uikit_tvos_input.rs (diag branch only). */
#define SDL_MAIN_HANDLED
#include "common.h"
#include <SDL3/SDL_main.h>
#import <GameController/GameController.h>

static const char *yes_no(bool b) { return b ? "yes" : "no"; }

static const char *ok(bool b)
{
    static char buf[512];
    if (b) {
        return "ok";
    }
    SDL_snprintf(buf, sizeof(buf), "error: %s", SDL_GetError());
    return buf;
}

/* ---- stand-ins for UIPress, UIKey and UISwipeGestureRecognizer */

@interface SDLTestKey : NSObject
@property(nonatomic) NSInteger keyCode;
@end
@implementation SDLTestKey
@end

@interface SDLTestPress : NSObject
@property(nonatomic, strong) id key;
@property(nonatomic) NSInteger type;
@property(nonatomic) NSTimeInterval timestamp;
@end
@implementation SDLTestPress
@end

@interface SDLTestSwipe : NSObject
@property(nonatomic) NSInteger state;
@property(nonatomic) NSUInteger direction;
@end
@implementation SDLTestSwipe
@end

/* ---- labels: joysticks by first appearance */

static SDL_JoystickID joys[16];
static int njoys;

static int joy_label(SDL_JoystickID id)
{
    for (int i = 0; i < njoys; i++) {
        if (joys[i] == id) {
            return i + 1;
        }
    }
    if (njoys < 16) {
        joys[njoys++] = id;
        return njoys;
    }
    return 0;
}

static SDL_WindowID window_id;

static const char *wl(SDL_WindowID id)
{
    return id == window_id ? "W" : id == 0 ? "0" : "?";
}

static const char *wname(Uint32 type)
{
    switch (type) {
#define N(x) case SDL_EVENT_##x: return #x;
    N(WINDOW_SHOWN) N(WINDOW_HIDDEN) N(WINDOW_EXPOSED) N(WINDOW_MOVED) N(WINDOW_RESIZED)
    N(WINDOW_PIXEL_SIZE_CHANGED) N(WINDOW_METAL_VIEW_RESIZED) N(WINDOW_MINIMIZED) N(WINDOW_MAXIMIZED)
    N(WINDOW_RESTORED) N(WINDOW_MOUSE_ENTER) N(WINDOW_MOUSE_LEAVE) N(WINDOW_FOCUS_GAINED)
    N(WINDOW_FOCUS_LOST) N(WINDOW_CLOSE_REQUESTED) N(WINDOW_HIT_TEST) N(WINDOW_ICCPROF_CHANGED)
    N(WINDOW_DISPLAY_CHANGED) N(WINDOW_DISPLAY_SCALE_CHANGED) N(WINDOW_SAFE_AREA_CHANGED)
    N(WINDOW_OCCLUDED) N(WINDOW_ENTER_FULLSCREEN) N(WINDOW_LEAVE_FULLSCREEN) N(WINDOW_DESTROYED)
    N(WINDOW_HDR_STATE_CHANGED)
#undef N
    default: return event_name(type);
    }
}

static const char *describe_i(const SDL_Event *e)
{
    static char buf[512];
    const char *n = event_name(e->type);
    switch (e->type) {
    case SDL_EVENT_MOUSE_ADDED: case SDL_EVENT_MOUSE_REMOVED:
        return NULL;
    case SDL_EVENT_KEY_DOWN: case SDL_EVENT_KEY_UP:
        SDL_snprintf(buf, sizeof(buf), "%s(%s scancode=%d key=0x%x down=%d w=%s)", e->type == SDL_EVENT_KEY_DOWN ? "KEY_DOWN" : "KEY_UP",
                     e->key.which == 0 ? "global" : "keyboard", (int)e->key.scancode, (unsigned)e->key.key, e->key.down ? 1 : 0, wl(e->key.windowID));
        return buf;
    case SDL_EVENT_TEXT_INPUT:
        SDL_snprintf(buf, sizeof(buf), "TEXT_INPUT(%s w=%s)", e->text.text, wl(e->text.windowID));
        return buf;
    case SDL_EVENT_TEXT_EDITING:
        SDL_snprintf(buf, sizeof(buf), "TEXT_EDITING(%s %d %d w=%s)", e->edit.text, (int)e->edit.start, (int)e->edit.length, wl(e->edit.windowID));
        return buf;
    case SDL_EVENT_SCREEN_KEYBOARD_SHOWN: return "SCREEN_KEYBOARD_SHOWN";
    case SDL_EVENT_SCREEN_KEYBOARD_HIDDEN: return "SCREEN_KEYBOARD_HIDDEN";
    case SDL_EVENT_KEYBOARD_ADDED: return "KEYBOARD_ADDED";
    case SDL_EVENT_KEYBOARD_REMOVED: return "KEYBOARD_REMOVED";
    case SDL_EVENT_JOYSTICK_ADDED: case SDL_EVENT_JOYSTICK_REMOVED:
        SDL_snprintf(buf, sizeof(buf), "%s(#%d)", e->type == SDL_EVENT_JOYSTICK_ADDED ? "JOYSTICK_ADDED" : "JOYSTICK_REMOVED", joy_label(e->jdevice.which));
        return buf;
    case SDL_EVENT_GAMEPAD_ADDED: case SDL_EVENT_GAMEPAD_REMOVED:
        SDL_snprintf(buf, sizeof(buf), "%s(#%d)", e->type == SDL_EVENT_GAMEPAD_ADDED ? "GAMEPAD_ADDED" : "GAMEPAD_REMOVED", joy_label(e->gdevice.which));
        return buf;
    case SDL_EVENT_JOYSTICK_AXIS_MOTION:
        SDL_snprintf(buf, sizeof(buf), "JOYSTICK_AXIS_MOTION(#%d axis=%d value=%d)", joy_label(e->jaxis.which), (int)e->jaxis.axis, (int)e->jaxis.value);
        return buf;
    case SDL_EVENT_JOYSTICK_BUTTON_DOWN: case SDL_EVENT_JOYSTICK_BUTTON_UP:
        SDL_snprintf(buf, sizeof(buf), "%s(#%d button=%d)", e->type == SDL_EVENT_JOYSTICK_BUTTON_DOWN ? "JOYSTICK_BUTTON_DOWN" : "JOYSTICK_BUTTON_UP", joy_label(e->jbutton.which), (int)e->jbutton.button);
        return buf;
    case SDL_EVENT_JOYSTICK_HAT_MOTION:
        SDL_snprintf(buf, sizeof(buf), "JOYSTICK_HAT_MOTION(#%d hat=%d value=%d)", joy_label(e->jhat.which), (int)e->jhat.hat, (int)e->jhat.value);
        return buf;
    case SDL_EVENT_GAMEPAD_AXIS_MOTION:
        SDL_snprintf(buf, sizeof(buf), "GAMEPAD_AXIS_MOTION(#%d %s=%d)", joy_label(e->gaxis.which), SDL_GetGamepadStringForAxis((SDL_GamepadAxis)e->gaxis.axis), (int)e->gaxis.value);
        return buf;
    case SDL_EVENT_GAMEPAD_BUTTON_DOWN: case SDL_EVENT_GAMEPAD_BUTTON_UP:
        SDL_snprintf(buf, sizeof(buf), "%s(#%d %s)", e->type == SDL_EVENT_GAMEPAD_BUTTON_DOWN ? "GAMEPAD_BUTTON_DOWN" : "GAMEPAD_BUTTON_UP", joy_label(e->gbutton.which), SDL_GetGamepadStringForButton((SDL_GamepadButton)e->gbutton.button));
        return buf;
    case SDL_EVENT_JOYSTICK_UPDATE_COMPLETE: case SDL_EVENT_GAMEPAD_UPDATE_COMPLETE:
        return NULL;
    default:
        break;
    }
    if (e->type >= SDL_EVENT_WINDOW_FIRST && e->type <= SDL_EVENT_WINDOW_LAST) {
        SDL_snprintf(buf, sizeof(buf), "%s:%s(%d,%d)", wl(e->window.windowID), wname(e->type), (int)e->window.data1, (int)e->window.data2);
        return buf;
    }
    return n;
}

static void take_queued(List *l)
{
    SDL_Event e;
    SDL_PumpEvents();
    while (SDL_PollEvent(&e)) {
        const char *d = describe_i(&e);
        if (d) {
            list_add(l, d);
        }
    }
}

static void events(const char *label, int ms)
{
    List l;
    memset(&l, 0, sizeof(l));
    take_queued(&l);
    for (int i = 0; i < ms / 10; i++) {
        SDL_Delay(10);
        take_queued(&l);
    }
    t("%s: events: %s", label, list_str(&l));
}

static void keyboards(const char *label)
{
    int n = 0;
    SDL_KeyboardID *kbs = SDL_GetKeyboards(&n);
    List l;
    memset(&l, 0, sizeof(l));
    for (int i = 0; kbs && i < n; i++) {
        list_add(&l, SDL_GetKeyboardNameForID(kbs[i]));
    }
    SDL_free(kbs);
    t("%s: keyboards: %s; has keyboard=%s", label, list_str(&l), yes_no(SDL_HasKeyboard()));
}

static void joysticks(const char *label)
{
    int n = 0;
    SDL_JoystickID *js = SDL_GetJoysticks(&n);
    List l;
    memset(&l, 0, sizeof(l));
    for (int i = 0; js && i < n; i++) {
        char item[256];
        SDL_snprintf(item, sizeof(item), "#%d %s", joy_label(js[i]), SDL_GetJoystickNameForID(js[i]));
        list_add(&l, item);
    }
    SDL_free(js);
    t("%s: joysticks: %s", label, list_str(&l));
}

static UIView *view;

static void press(const char *label, NSInteger code, NSInteger type)
{
    SDLTestPress *p = [[SDLTestPress alloc] init];
    if (code >= 0) {
        SDLTestKey *key = [[SDLTestKey alloc] init];
        key.keyCode = code;
        p.key = key;
    }
    p.type = type;
    p.timestamp = 1.0;
    NSSet *set = [NSSet setWithObject:p];
    char l[128];
    [view pressesBegan:(NSSet<UIPress *> *)set withEvent:(UIPressesEvent *)p];
    SDL_snprintf(l, sizeof(l), "%s began", label);
    events(l, 0);
    [view pressesEnded:(NSSet<UIPress *> *)set withEvent:(UIPressesEvent *)p];
    SDL_snprintf(l, sizeof(l), "%s ended", label);
    events(l, 0);
}

static void presses(const char *label)
{
    struct { const char *name; NSInteger code, type; } list[] = {
        { "up arrow", -1, UIPressTypeUpArrow }, { "down arrow", -1, UIPressTypeDownArrow },
        { "left arrow", -1, UIPressTypeLeftArrow }, { "right arrow", -1, UIPressTypeRightArrow },
        { "select", -1, UIPressTypeSelect }, { "menu", -1, UIPressTypeMenu },
        { "play/pause", -1, UIPressTypePlayPause }, { "page up", -1, UIPressTypePageUp },
        { "key a", 4, UIPressTypeSelect },
    };
    for (size_t i = 0; i < SDL_arraysize(list); i++) {
        char l[128];
        SDL_snprintf(l, sizeof(l), "%s: press %s", label, list[i].name);
        press(l, list[i].code, list[i].type);
    }
}

static void swipes(const char *label)
{
    struct { const char *name; NSUInteger direction; NSInteger state; } list[] = {
        { "up", UISwipeGestureRecognizerDirectionUp, UIGestureRecognizerStateEnded },
        { "down", UISwipeGestureRecognizerDirectionDown, UIGestureRecognizerStateEnded },
        { "left", UISwipeGestureRecognizerDirectionLeft, UIGestureRecognizerStateEnded },
        { "right", UISwipeGestureRecognizerDirectionRight, UIGestureRecognizerStateEnded },
        { "up (began)", UISwipeGestureRecognizerDirectionUp, UIGestureRecognizerStateBegan },
    };
    for (size_t i = 0; i < SDL_arraysize(list); i++) {
        SDLTestSwipe *s = [[SDLTestSwipe alloc] init];
        s.direction = list[i].direction;
        s.state = list[i].state;
        [view performSelector:@selector(swipeGesture:) withObject:s];
        char l[128];
        SDL_snprintf(l, sizeof(l), "%s: swipe %s", label, list[i].name);
        events(l, 0);
    }
}

static void describe_joystick(SDL_JoystickID id)
{
    int l = joy_label(id);
    char guid[64];
    SDL_GUIDToString(SDL_GetJoystickGUIDForID(id), guid, sizeof(guid));
    t("#%d: name=%s guid=%s vendor=0x%04x product=0x%04x gamepad=%s type=%d", l, SDL_GetJoystickNameForID(id), guid,
      SDL_GetJoystickVendorForID(id), SDL_GetJoystickProductForID(id), yes_no(SDL_IsGamepad(id)), (int)SDL_GetGamepadTypeForID(id));
    char *mapping = SDL_GetGamepadMappingForID(id);
    t("#%d: mapping=%s", l, mapping ? mapping : "(null)");
    SDL_free(mapping);
}

static SDL_Gamepad *open_gamepad(SDL_JoystickID id)
{
    int l = joy_label(id);
    SDL_Gamepad *gp = SDL_OpenGamepad(id);
    if (!gp) {
        t("#%d: open: error: %s", l, SDL_GetError());
        return NULL;
    }
    SDL_Joystick *j = SDL_GetGamepadJoystick(gp);
    t("#%d: open: axes=%d buttons=%d hats=%d", l, SDL_GetNumJoystickAxes(j), SDL_GetNumJoystickButtons(j), SDL_GetNumJoystickHats(j));
    List b;
    memset(&b, 0, sizeof(b));
    for (int i = 0; i < SDL_GAMEPAD_BUTTON_COUNT; i++) {
        if (SDL_GamepadHasButton(gp, (SDL_GamepadButton)i)) {
            list_add(&b, SDL_GetGamepadStringForButton((SDL_GamepadButton)i));
        }
    }
    t("#%d: buttons: %s", l, list_str(&b));
    return gp;
}

static void micro_state(const char *label, GCController *c)
{
    t("%s: allows rotation=%s absolute d-pad=%s", label, yes_no(c.microGamepad.allowsRotation),
      yes_no(c.microGamepad.reportsAbsoluteDpadValues));
}

static SDL_JoystickID wait_for_new(int seen)
{
    Uint64 end = SDL_GetTicks() + 5000;
    while (SDL_GetTicks() < end) {
        int n = 0;
        SDL_JoystickID *js = SDL_GetJoysticks(&n);
        for (int i = 0; js && i < n; i++) {
            bool old = false;
            for (int k = 0; k < seen; k++) {
                if (joys[k] == js[i]) old = true;
            }
            if (!old) {
                SDL_JoystickID id = js[i];
                SDL_free(js);
                return id;
            }
        }
        SDL_free(js);
        SDL_Delay(10);
        SDL_PumpEvents();
    }
    return 0;
}

static int app_main(int argc, char *argv[])
{
    (void)argc; (void)argv;
    printf("uikit_tvos_input_ref: %.3f s: main\n", elapsed());
    bool b = SDL_Init(SDL_INIT_VIDEO | SDL_INIT_GAMEPAD);
    t("init video and gamepad: %s", ok(b));
    List ignored;
    Uint64 end = SDL_GetTicks() + 30000;
    while ([UIApplication sharedApplication].applicationState != UIApplicationStateActive && SDL_GetTicks() < end) {
        memset(&ignored, 0, sizeof(ignored)); take_queued(&ignored);
        SDL_Delay(10);
    }
    for (int i = 0; i < 50; i++) {
        memset(&ignored, 0, sizeof(ignored)); take_queued(&ignored);
        SDL_Delay(10);
    }
    t("application state: %d", (int)[UIApplication sharedApplication].applicationState);
    keyboards("start");
    joysticks("start");
    GCKeyboard *coalesced = [GCKeyboard coalescedKeyboard];
    t("coalesced keyboard: %s", yes_no(coalesced != nil));
    if (coalesced) {
        [[NSNotificationCenter defaultCenter] postNotificationName:GCKeyboardDidDisconnectNotification object:coalesced];
        events("keyboard disconnected", 0);
        keyboards("keyboard disconnected");
    }

    SDL_Window *w = SDL_CreateWindow("W", 640, 480, 0);
    t("W: create: %s", ok(w != NULL));
    window_id = SDL_GetWindowID(w);
    events("W: created", 500);
    UIWindow *uw = (__bridge UIWindow *)SDL_GetPointerProperty(SDL_GetWindowProperties(w), SDL_PROP_WINDOW_UIKIT_WINDOW_POINTER, NULL);
    view = uw.rootViewController.view;
    t("W: view %s", object_getClassName(view));

    presses("no remote");
    swipes("no remote");

    /* Text input: the hidden text field */
    t("start text input: %s", ok(SDL_StartTextInput(w)));
    events("text input started", 300);
    t("text input active=%s", yes_no(SDL_TextInputActive(w)));
    UITextField *tf = nil;
    for (UIView *sub in view.subviews) {
        if ([sub isKindOfClass:[UITextField class]]) {
            tf = (UITextField *)sub;
        }
    }
    t("text field: %s first responder=%s", tf ? object_getClassName(tf) : "nil", yes_no(tf.isFirstResponder));
    [tf insertText:@"tv"];
    events("insert \"tv\"", 0);
    [tf deleteBackward];
    events("delete backward", 0);
    t("stop text input: %s", ok(SDL_StopTextInput(w)));
    events("text input stopped", 300);
    t("text input active=%s first responder=%s", yes_no(SDL_TextInputActive(w)), yes_no(tf.isFirstResponder));

    /* The Siri Remote as a joystick: a stand-in micro gamepad */
    GCController *micro = [GCController controllerWithMicroGamepad];
    t("micro snapshot: vendor=%s category=%s micro=%s", micro.vendorName.UTF8String, micro.productCategory.UTF8String, yes_no(micro.microGamepad != nil));
    int seen = njoys;
    [[NSNotificationCenter defaultCenter] postNotificationName:GCControllerDidConnectNotification object:micro];
    events("micro connected", 100);
    SDL_JoystickID id = wait_for_new(seen);
    SDL_Gamepad *gp = NULL;
    if (id) {
        describe_joystick(id);
        gp = open_gamepad(id);
        events("opened", 100);
        micro_state("opened", micro);
    } else {
        t("micro: no joystick");
    }
    presses("remote open");
    swipes("remote open");
    GCMicroGamepad *mg = micro.microGamepad;
    [mg.dpad setValueForXAxis:0.5f yAxis:-1.0f];
    events("d-pad", 100);
    [mg.dpad setValueForXAxis:0.0f yAxis:0.0f];
    [mg.buttonA setValue:1.0f];
    events("button A", 100);
    [mg.buttonA setValue:0.0f];
    [mg.buttonX setValue:1.0f];
    events("button X", 100);
    [mg.buttonX setValue:0.0f];
    [mg.buttonMenu setValue:1.0f];
    events("menu", 100);
    [mg.buttonMenu setValue:0.0f];
    events("released", 100);
    if (gp) {
        SDL_CloseGamepad(gp);
        t("closed");
    }
    presses("remote closed");
    SDL_SetHint(SDL_HINT_APPLE_TV_REMOTE_ALLOW_ROTATION, "1");
    if (id) {
        gp = open_gamepad(id);
        micro_state("rotation hint", micro);
        SDL_CloseGamepad(gp);
    }
    SDL_SetHint(SDL_HINT_APPLE_TV_REMOTE_ALLOW_ROTATION, NULL);
    [[NSNotificationCenter defaultCenter] postNotificationName:GCControllerDidDisconnectNotification object:micro];
    events("micro disconnected", 100);
    joysticks("micro disconnected");

    /* Not as a joystick */
    SDL_SetHint(SDL_HINT_TV_REMOTE_AS_JOYSTICK, "0");
    GCController *micro2 = [GCController controllerWithMicroGamepad];
    [[NSNotificationCenter defaultCenter] postNotificationName:GCControllerDidConnectNotification object:micro2];
    events("micro (not a joystick) connected", 300);
    joysticks("micro (not a joystick) connected");
    [[NSNotificationCenter defaultCenter] postNotificationName:GCControllerDidDisconnectNotification object:micro2];
    events("micro (not a joystick) disconnected", 100);
    SDL_SetHint(SDL_HINT_TV_REMOTE_AS_JOYSTICK, NULL);

    SDL_DestroyWindow(w);
    events("W: destroyed", 300);
    SDL_Quit();
    t("quit");
    exit_line(0);
    exit(0);
}

int main(int argc, char *argv[])
{
    elapsed();
    int status = SDL_RunApp(argc, argv, app_main, NULL);
    printf("uikit_tvos_input_ref: SDL_RunApp returned %d\n", status);
    exit_line(1);
    return 1;
}
