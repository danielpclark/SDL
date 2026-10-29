package org.libsdl.inmem;

public class SDLInMemoryThread extends Thread {
    public SDLInMemoryThread() { super(); }
    public native void run();
}
