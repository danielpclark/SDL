// The reference for dexgen.py's classes.dex: compiled with javac and d8 in
// the spike job only, to compare dexdump output (never packaged).
package org.libsdl.app;

public class SDLActivity extends android.app.Activity {
    static { System.loadLibrary("main"); }
    public SDLActivity() { super(); }
    protected native void onCreate(android.os.Bundle b);
    protected native void onResume();
    protected native void onPause();
    protected native void onDestroy();
    public native void onWindowFocusChanged(boolean f);
    public native boolean dispatchTouchEvent(android.view.MotionEvent e);
    public native boolean dispatchKeyEvent(android.view.KeyEvent e);
    public native boolean dispatchGenericMotionEvent(android.view.MotionEvent e);
}
