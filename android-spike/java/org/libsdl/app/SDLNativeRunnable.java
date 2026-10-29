package org.libsdl.app;

public class SDLNativeRunnable implements Runnable {
    public long ptr;
    public SDLNativeRunnable() { super(); }
    public native void run();
}
