package org.libsdl.app;

public class SDLSurface extends android.view.SurfaceView implements android.view.SurfaceHolder.Callback {
    public SDLSurface(android.content.Context c) { super(c); }
    public native void surfaceCreated(android.view.SurfaceHolder h);
    public native void surfaceChanged(android.view.SurfaceHolder h, int f, int w, int hh);
    public native void surfaceDestroyed(android.view.SurfaceHolder h);
}
