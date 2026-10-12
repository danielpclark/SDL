// Generate sdl3/src/joystick/testdata/mfi_mapping.txt, the expected results
// of sdl3/src/joystick/apple/tests.rs: upstream's ElementAlreadyHandled(),
// the axes/buttons selection of IOS_AddMFIJoystickDevice() and
// IOS_JoystickGetGamepadMapping(), copied from SDL_mfijoystick.m (SDL
// ea7a2dab) with the GCControllerElement class checks replaced by a kind
// letter per element: 'a' analog axis, 'B' analog button, 'b' digital
// button, 'd' direction pad. The profiles are
// sdl3/src/joystick/testdata/mfi_profiles.txt (label|flags|elements).
//
// Usage (macOS):
//   clang -fobjc-arc -framework Foundation -framework GameController \
//       tools/gen_mfi_mapping_testdata.m -o gen_mfi_mapping_testdata
//   ./gen_mfi_mapping_testdata sdl3/src/joystick/testdata/mfi_profiles.txt \
//       > sdl3/src/joystick/testdata/mfi_mapping.txt
#import <Foundation/Foundation.h>
#import <GameController/GameController.h>

typedef struct {
    bool is_siri_remote_flag;
    int is_siri_remote;
    bool is_switch_joycon_pair;
    bool is_backbone_one;
    bool has_nintendo_buttons;
    bool is_switch_joyconL;
    bool is_switch_joyconR;
    NSArray *axes;
    NSArray *buttons;
} Device;

typedef enum { EMappingKind_None, EMappingKind_Button, EMappingKind_Axis, EMappingKind_Hat } EMappingKind;
typedef struct { EMappingKind kind; int target; bool axis_reversed, half_axis_positive, half_axis_negative; } SDL_InputMapping;
typedef struct {
    SDL_InputMapping a, b, x, y, back, guide, start, leftstick, rightstick, leftshoulder, rightshoulder,
        dpup, dpdown, dpleft, dpright, misc1, misc2, misc3, misc4, misc5, misc6,
        right_paddle1, left_paddle1, right_paddle2, left_paddle2,
        leftx, lefty, rightx, righty, lefttrigger, righttrigger, touchpad;
} SDL_GamepadMapping;

static bool ElementAlreadyHandled(Device *device, NSString *element, NSDictionary *elements)
{
    if ([element isEqualToString:@"Left Thumbstick Left"] ||
        [element isEqualToString:@"Left Thumbstick Right"]) {
        if (elements[@"Left Thumbstick X Axis"]) {
            return true;
        }
    }
    if ([element isEqualToString:@"Left Thumbstick Up"] ||
        [element isEqualToString:@"Left Thumbstick Down"]) {
        if (elements[@"Left Thumbstick Y Axis"]) {
            return true;
        }
    }
    if ([element isEqualToString:@"Right Thumbstick Left"] ||
        [element isEqualToString:@"Right Thumbstick Right"]) {
        if (elements[@"Right Thumbstick X Axis"]) {
            return true;
        }
    }
    if ([element isEqualToString:@"Right Thumbstick Up"] ||
        [element isEqualToString:@"Right Thumbstick Down"]) {
        if (elements[@"Right Thumbstick Y Axis"]) {
            return true;
        }
    }
    if (device->is_siri_remote) {
        if ([element isEqualToString:@"Direction Pad Left"] ||
            [element isEqualToString:@"Direction Pad Right"]) {
            if (elements[@"Direction Pad X Axis"]) {
                return true;
            }
        }
        if ([element isEqualToString:@"Direction Pad Up"] ||
            [element isEqualToString:@"Direction Pad Down"]) {
            if (elements[@"Direction Pad Y Axis"]) {
                return true;
            }
        }
    } else {
        if ([element isEqualToString:@"Direction Pad X Axis"]) {
            if (elements[@"Direction Pad Left"] &&
                elements[@"Direction Pad Right"]) {
                return true;
            }
        }
        if ([element isEqualToString:@"Direction Pad Y Axis"]) {
            if (elements[@"Direction Pad Up"] &&
                elements[@"Direction Pad Down"]) {
                return true;
            }
        }
    }
    if ([element isEqualToString:@"Cardinal Direction Pad X Axis"]) {
        if (elements[@"Cardinal Direction Pad Left"] &&
            elements[@"Cardinal Direction Pad Right"]) {
            return true;
        }
    }
    if ([element isEqualToString:@"Cardinal Direction Pad Y Axis"]) {
        if (elements[@"Cardinal Direction Pad Up"] &&
            elements[@"Cardinal Direction Pad Down"]) {
            return true;
        }
    }
    if ([element isEqualToString:@"Touchpad 1 X Axis"] ||
        [element isEqualToString:@"Touchpad 1 Y Axis"] ||
        [element isEqualToString:@"Touchpad 1 Left"] ||
        [element isEqualToString:@"Touchpad 1 Right"] ||
        [element isEqualToString:@"Touchpad 1 Up"] ||
        [element isEqualToString:@"Touchpad 1 Down"] ||
        [element isEqualToString:@"Touchpad 2 X Axis"] ||
        [element isEqualToString:@"Touchpad 2 Y Axis"] ||
        [element isEqualToString:@"Touchpad 2 Left"] ||
        [element isEqualToString:@"Touchpad 2 Right"] ||
        [element isEqualToString:@"Touchpad 2 Up"] ||
        [element isEqualToString:@"Touchpad 2 Down"]) {
        // The touchpad is handled separately
        return true;
    }
    if ([element isEqualToString:@"Button Home"]) {
        if (device->is_switch_joycon_pair) {
            // The Nintendo Switch JoyCon home button doesn't ever show as being held down
            return true;
        }
    }
    if ([element isEqualToString:@"Button Share"]) {
        if (device->is_backbone_one) {
            // The Backbone app uses share button
            return true;
        }
    }
    return false;
}

static void Select(Device *device, NSDictionary *elements)
{
    NSArray *axes = [[[elements allKeys] sortedArrayUsingSelector:@selector(caseInsensitiveCompare:)]
                                     filteredArrayUsingPredicate:[NSPredicate predicateWithBlock:^BOOL(id object, NSDictionary *bindings) {
        if (ElementAlreadyHandled(device, (NSString *)object, elements)) {
            return false;
        }
        NSString *kind = elements[object];
        if ([kind isEqualToString:@"a"] || [kind isEqualToString:@"B"]) {
            return true;
        }
        return false;
    }]];
    NSArray *buttons = [[[elements allKeys] sortedArrayUsingSelector:@selector(caseInsensitiveCompare:)]
                                        filteredArrayUsingPredicate:[NSPredicate predicateWithBlock:^BOOL(id object, NSDictionary *bindings) {
        if (ElementAlreadyHandled(device, (NSString *)object, elements)) {
            return false;
        }
        NSString *kind = elements[object];
        if ([kind isEqualToString:@"B"] || [kind isEqualToString:@"b"]) {
            return true;
        }
        return false;
    }]];
    device->axes = axes;
    device->buttons = buttons;
}

static bool GetGamepadMapping(Device *device, SDL_GamepadMapping *out)
{
        int axis = 0;
        for (id key in device->axes) {
            if ([(NSString *)key isEqualToString:@"Left Thumbstick X Axis"] ||
                [(NSString *)key isEqualToString:@"Direction Pad X Axis"]) {
                out->leftx.kind = EMappingKind_Axis;
                out->leftx.target = axis;
            } else if ([(NSString *)key isEqualToString:@"Left Thumbstick Y Axis"] ||
                       [(NSString *)key isEqualToString:@"Direction Pad Y Axis"]) {
                out->lefty.kind = EMappingKind_Axis;
                out->lefty.target = axis;
                out->lefty.axis_reversed = true;
            } else if ([(NSString *)key isEqualToString:@"Right Thumbstick X Axis"]) {
                out->rightx.kind = EMappingKind_Axis;
                out->rightx.target = axis;
            } else if ([(NSString *)key isEqualToString:@"Right Thumbstick Y Axis"]) {
                out->righty.kind = EMappingKind_Axis;
                out->righty.target = axis;
                out->righty.axis_reversed = true;
            } else if ([(NSString *)key isEqualToString:GCInputLeftTrigger]) {
                out->lefttrigger.kind = EMappingKind_Axis;
                out->lefttrigger.target = axis;
                out->lefttrigger.half_axis_positive = true;
            } else if ([(NSString *)key isEqualToString:GCInputRightTrigger]) {
                out->righttrigger.kind = EMappingKind_Axis;
                out->righttrigger.target = axis;
                out->righttrigger.half_axis_positive = true;
            }
            ++axis;
        }

        int button = 0;
        for (id key in device->buttons) {
            SDL_InputMapping *mapping = NULL;

            if ([(NSString *)key isEqualToString:GCInputButtonA]) {
                if (device->is_siri_remote > 1) {
                    // GCInputButtonA is triggered for any D-Pad press, ignore it in favor of "Button Center"
                } else if (device->has_nintendo_buttons) {
                    mapping = &out->b;
                } else {
                    mapping = &out->a;
                }
            } else if ([(NSString *)key isEqualToString:GCInputButtonB]) {
                if (device->has_nintendo_buttons) {
                    mapping = &out->a;
                } else if (device->is_switch_joyconL || device->is_switch_joyconR) {
                    mapping = &out->x;
                } else {
                    mapping = &out->b;
                }
            } else if ([(NSString *)key isEqualToString:GCInputButtonX]) {
                if (device->has_nintendo_buttons) {
                    mapping = &out->y;
                } else if (device->is_switch_joyconL || device->is_switch_joyconR) {
                    mapping = &out->b;
                } else {
                    mapping = &out->x;
                }
            } else if ([(NSString *)key isEqualToString:GCInputButtonY]) {
                if (device->has_nintendo_buttons) {
                    mapping = &out->x;
                } else {
                    mapping = &out->y;
                }
            } else if ([(NSString *)key isEqualToString:@"Direction Pad Left"]) {
                mapping = &out->dpleft;
            } else if ([(NSString *)key isEqualToString:@"Direction Pad Right"]) {
                mapping = &out->dpright;
            } else if ([(NSString *)key isEqualToString:@"Direction Pad Up"]) {
                mapping = &out->dpup;
            } else if ([(NSString *)key isEqualToString:@"Direction Pad Down"]) {
                mapping = &out->dpdown;
            } else if ([(NSString *)key isEqualToString:@"Cardinal Direction Pad Left"]) {
                mapping = &out->dpleft;
            } else if ([(NSString *)key isEqualToString:@"Cardinal Direction Pad Right"]) {
                mapping = &out->dpright;
            } else if ([(NSString *)key isEqualToString:@"Cardinal Direction Pad Up"]) {
                mapping = &out->dpup;
            } else if ([(NSString *)key isEqualToString:@"Cardinal Direction Pad Down"]) {
                mapping = &out->dpdown;
            } else if ([(NSString *)key isEqualToString:GCInputLeftShoulder]) {
                mapping = &out->leftshoulder;
            } else if ([(NSString *)key isEqualToString:GCInputRightShoulder]) {
                mapping = &out->rightshoulder;
            } else if ([(NSString *)key isEqualToString:GCInputLeftThumbstickButton]) {
                mapping = &out->leftstick;
            } else if ([(NSString *)key isEqualToString:GCInputRightThumbstickButton]) {
                mapping = &out->rightstick;
            } else if ([(NSString *)key isEqualToString:@"Button Home"]) {
                mapping = &out->guide;
            } else if ([(NSString *)key isEqualToString:GCInputButtonMenu]) {
                if (device->is_siri_remote) {
                    mapping = &out->b;
                } else {
                    mapping = &out->start;
                }
            } else if ([(NSString *)key isEqualToString:GCInputButtonOptions]) {
                mapping = &out->back;
            } else if ([(NSString *)key isEqualToString:@"Button Share"]) {
                mapping = &out->misc1;
            } else if ([(NSString *)key isEqualToString:GCInputXboxPaddleOne]) {
                mapping = &out->right_paddle1;
            } else if ([(NSString *)key isEqualToString:GCInputXboxPaddleTwo]) {
                mapping = &out->right_paddle2;
            } else if ([(NSString *)key isEqualToString:GCInputXboxPaddleThree]) {
                mapping = &out->left_paddle1;
            } else if ([(NSString *)key isEqualToString:GCInputXboxPaddleFour]) {
                mapping = &out->left_paddle2;
            } else if ([(NSString *)key isEqualToString:GCInputLeftTrigger]) {
                mapping = &out->lefttrigger;
            } else if ([(NSString *)key isEqualToString:GCInputRightTrigger]) {
                mapping = &out->righttrigger;
            } else if ([(NSString *)key isEqualToString:GCInputDualShockTouchpadButton]) {
                mapping = &out->touchpad;
            } else if ([(NSString *)key isEqualToString:@"Button Center"]) {
                mapping = &out->a;
            }
            if (mapping && mapping->kind == EMappingKind_None) {
                mapping->kind = EMappingKind_Button;
                mapping->target = button;
            }
            ++button;
        }

        return true;
}

static void PrintMapping(const char *name, SDL_InputMapping m)
{
    if (m.kind != EMappingKind_None) {
        printf(" %s:%c%d%s%s%s", name, m.kind == EMappingKind_Button ? 'b' : 'a', m.target,
               m.axis_reversed ? "~" : "", m.half_axis_positive ? "+" : "", m.half_axis_negative ? "-" : "");
    }
}

static void Run(const char *label, Device device, NSString *spec)
{
    NSMutableDictionary *elements = [NSMutableDictionary dictionary];
    for (NSString *item in [spec componentsSeparatedByString:@";"]) {
        if (item.length == 0) {
            continue;
        }
        NSRange colon = [item rangeOfString:@":" options:NSBackwardsSearch];
        elements[[item substringToIndex:colon.location]] = [item substringFromIndex:colon.location + 1];
    }
    Select(&device, elements);
    printf("%s axes:", label);
    for (NSString *a in device.axes) {
        printf(" [%s]", a.UTF8String);
    }
    printf("\n%s buttons:", label);
    for (NSString *b in device.buttons) {
        printf(" [%s]", b.UTF8String);
    }
    SDL_GamepadMapping m;
    memset(&m, 0, sizeof(m));
    GetGamepadMapping(&device, &m);
    printf("\n%s mapping:", label);
#define P(f) PrintMapping(#f, m.f)
    P(a); P(b); P(x); P(y); P(back); P(guide); P(start); P(leftstick); P(rightstick);
    P(leftshoulder); P(rightshoulder); P(dpup); P(dpdown); P(dpleft); P(dpright);
    P(misc1); P(misc2); P(misc3); P(misc4); P(misc5); P(misc6);
    P(right_paddle1); P(left_paddle1); P(right_paddle2); P(left_paddle2);
    P(leftx); P(lefty); P(rightx); P(righty); P(lefttrigger); P(righttrigger); P(touchpad);
    printf("\n");
}

int main(int argc, char **argv)
{
    @autoreleasepool {
        // One line per profile: label|flags|elements, from the profiles file.
        NSString *text = [NSString stringWithContentsOfFile:@(argv[1]) encoding:NSUTF8StringEncoding error:nil];
        for (NSString *line in [text componentsSeparatedByString:@"\n"]) {
            NSArray *parts = [line componentsSeparatedByString:@"|"];
            if (parts.count != 3) {
                continue;
            }
            Device device = { 0 };
            for (NSString *flag in [parts[1] componentsSeparatedByString:@","]) {
                if ([flag hasPrefix:@"siri="]) {
                    device.is_siri_remote = [[flag substringFromIndex:5] intValue];
                } else if ([flag isEqualToString:@"joycon_pair"]) {
                    device.is_switch_joycon_pair = true;
                } else if ([flag isEqualToString:@"backbone"]) {
                    device.is_backbone_one = true;
                } else if ([flag isEqualToString:@"nintendo"]) {
                    device.has_nintendo_buttons = true;
                } else if ([flag isEqualToString:@"joyconL"]) {
                    device.is_switch_joyconL = true;
                } else if ([flag isEqualToString:@"joyconR"]) {
                    device.is_switch_joyconR = true;
                }
            }
            Run([parts[0] UTF8String], device, parts[2]);
        }
    }
    return 0;
}
