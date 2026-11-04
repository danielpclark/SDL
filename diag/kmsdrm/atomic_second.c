/* diag only: upstream with atomic commits and a window on the second
 * display of a two-pipeline vkms device (the first window there: its
 * display's plane is set up), then a window on the first display and one
 * on the second (the second display's plane never set up). Not a twin. */
#include "kms_common.h"

static void one(const char *card, int which[], int n)
{
    int nd = 0;
    SDL_DisplayID *ids;
    SDL_Window *w[2] = { 0 };
    SDL_GLContext c[2] = { 0 };
    if (!init_video()) {
        return;
    }
    ids = SDL_GetDisplays(&nd);
    for (int i = 0; i < n; i++) {
        SDL_PropertiesID p = SDL_CreateProperties();
        SDL_SetNumberProperty(p, SDL_PROP_WINDOW_CREATE_X_NUMBER, SDL_WINDOWPOS_UNDEFINED_DISPLAY(ids[which[i]]));
        SDL_SetNumberProperty(p, SDL_PROP_WINDOW_CREATE_Y_NUMBER, SDL_WINDOWPOS_UNDEFINED_DISPLAY(ids[which[i]]));
        SDL_SetNumberProperty(p, SDL_PROP_WINDOW_CREATE_WIDTH_NUMBER, 640);
        SDL_SetNumberProperty(p, SDL_PROP_WINDOW_CREATE_HEIGHT_NUMBER, 480);
        w[i] = SDL_CreateWindowWithProperties(p);
        SDL_DestroyProperties(p);
        line("window on display %d: %s", which[i], result(w[i] != NULL));
        c[i] = w[i] ? SDL_GL_CreateContext(w[i]) : NULL;
        line("context: %s", result(c[i] != NULL));
    }
    for (int i = 0; i < n; i++) {
        if (c[i]) {
            SDL_GL_MakeCurrent(w[i], c[i]);
            gl_clear(1, 0, 0);
            line("swap on display %d: %s", which[i], result(SDL_GL_SwapWindow(w[i])));
            gl_clear(0, 1, 0);
            line("swap on display %d: %s", which[i], result(SDL_GL_SwapWindow(w[i])));
        }
    }
    for (int i = 0; i < n; i++) {
        if (c[i]) SDL_GL_DestroyContext(c[i]);
        if (w[i]) SDL_DestroyWindow(w[i]);
    }
    SDL_free(ids);
    quit_video();
    (void)card;
}

int main(int argc, char **argv)
{
    char card[16] = "";
    int second[] = { 1 }, both[] = { 0, 1 };
    begin();
    request("vkms add multi 2");
    for (int i = 0; i < nreplies; i++) line("%s", replies[i]);
    if (nreplies < 1 || sscanf(replies[0], "vkms: added multi card%15s", card) != 1) return 1;
    SDL_SetHintWithPriority(SDL_HINT_KMSDRM_DEVICE_INDEX, card, SDL_HINT_OVERRIDE);
    if (argc > 1 && strcmp(argv[1], "both") == 0) {
        line("-- a window on each display (atomic)");
        one(card, both, 2);
    } else {
        line("-- a window on the second display alone (atomic)");
        one(card, second, 1);
    }
    ask("vkms remove multi");
    return finish("atomic_second");
}
