#!/usr/bin/env python3
"""Run a program built for wasm32-unknown-emscripten: Cargo's runner for the
target (.cargo/config.toml), for the test programs and the doctests.

Usage: emscripten_run.py PROGRAM.js [ARGUMENTS...]

By default the program runs under node, as `node PROGRAM.js ARGUMENTS...`
(this script then just execs node). In a browser it runs in headless
Chromium instead, when SDL3_EMSCRIPTEN_BROWSER is 1, or when it's unset and
the program asks for a browser (a custom section `sdl3_runner` in its .wasm
holding `browser`, as sdl3/tests/emscripten_browser.rs has) and a Chrome
or Chromium is found; SDL3_EMSCRIPTEN_BROWSER=0 always picks node. Then
this script

* serves the program's directory over HTTP on 127.0.0.1 (http.server), with
  a page generated at run time (an HTML shell in a temporary directory: the
  repository has no HTML or JavaScript of its own) that loads the program;
* starts Chromium headless with a temporary profile and drives it over the
  DevTools protocol on a pipe (--remote-debugging-pipe): the page gets the
  program's arguments (`Module.arguments`: a libtest program's filters and
  options, or a `harness = false` program's) and this script's environment
  (set in emscripten's `ENV` from `Module.preRun`, as node's environment is
  with -sNODE_HOST_ENV), before any of its scripts runs;
* copies what the program prints to stdout (`Module.print`, console.log)
  and stderr (`Module.printErr`, console.error and warnings, uncaught
  exceptions), and exits with the program's exit status, once emscripten's
  runtime has exited (`Module.onExit`). As under node, an `exit()` while the
  runtime is kept alive (a main loop, a pending timer) ends the run only
  once nothing keeps it alive any more; an uncaught exception or an abort
  ends it with 1.

Environment:
  SDL3_EMSCRIPTEN_BROWSER  1: run in the browser; 0: under node; unset or
                           empty: what the program asks for
  CHROME                   the Chrome or Chromium binary (else google-chrome,
                           google-chrome-stable, chromium, chromium-browser
                           or chrome in PATH, or the usual macOS
                           applications)
  SDL3_EMSCRIPTEN_NODE     the node binary (default: node, from PATH)
  SDL3_EMSCRIPTEN_TIMEOUT  seconds a browser run may take (default 900;
                           0: no limit)
  SDL3_EMSCRIPTEN_CHROME_FLAGS  more Chromium options (shell syntax)
  SDL3_EMSCRIPTEN_VERBOSE  1: the page's URL, the browser's own log and the
                           HTTP requests on stderr

Chromium runs with --headless=new --enable-unsafe-swiftshader
--use-angle=swiftshader (WebGL on SwiftShader, without a GPU)
--autoplay-policy=no-user-gesture-required (Web Audio plays without a
click) --use-fake-device-for-media-stream --use-fake-ui-for-media-stream
(a fake camera and microphone, allowed without asking), and with
--no-sandbox only where its sandbox can't start: as root, or after a first
start without it failed (containers without user namespaces). The browser
mode needs a POSIX system (the pipe is file descriptors 3 and 4).
"""
import fcntl
import functools
import http.server
import json
import os
import queue
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time

PROGRAM = os.path.basename(__file__)

# The page. `sdl3RunnerConfig` (the arguments and environment) and the
# `sdl3RunnerReport` binding are set up over the DevTools protocol before
# any of the page's scripts run.
SHELL = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>{title}</title>
<link rel="icon" href="data:,">
<style>body {{ margin: 0; background: #000; }} canvas {{ display: block; }}</style>
</head>
<body>
<canvas id="canvas" width="640" height="480" tabindex="-1" oncontextmenu="event.preventDefault()"></canvas>
<script>
var Module = (() => {{
  const config = globalThis.sdl3RunnerConfig;
  const report = (what, value) => globalThis.sdl3RunnerReport(JSON.stringify([what, value]));
  let draining = false;
  return {{
    arguments: config.arguments,
    canvas: document.getElementById('canvas'),
    print: (text) => console.log(text),
    printErr: (text) => console.error(text),
    preRun: [() => {{
      // (node's environment, as -sNODE_HOST_ENV passes it under node)
      Object.assign(ENV, config.env);
      // Every exit(): with the runtime kept alive it doesn't call onExit,
      // and under node the program ends once nothing is pending, so wait
      // for that.
      if (typeof quit_ === 'function') {{
        const quit = quit_;
        quit_ = (status, toThrow) => {{
          report('quit', status);
          if (!draining && typeof keepRuntimeAlive === 'function') {{
            draining = true;
            const poll = setInterval(() => {{
              if (!keepRuntimeAlive()) {{
                clearInterval(poll);
                report('drained', EXITSTATUS);
              }}
            }}, 20);
          }}
          quit(status, toThrow);
        }};
      }}
    }}],
    onExit: (status) => report('exit', status),
    onAbort: (what) => report('abort', String(what)),
  }};
}})();
</script>
<script src="{script}"></script>
</body>
</html>
"""
SHELL_PATH = "/__sdl3_runner__.html"

CHROME_FLAGS = [
    "--headless=new",
    "--enable-unsafe-swiftshader",
    "--use-angle=swiftshader",
    "--autoplay-policy=no-user-gesture-required",
    "--use-fake-device-for-media-stream",
    "--use-fake-ui-for-media-stream",
    "--no-first-run",
    "--no-default-browser-check",
    "--remote-debugging-pipe",
]

VERBOSE = os.environ.get("SDL3_EMSCRIPTEN_VERBOSE", "") not in ("", "0")


def note(message):
    print(f"{PROGRAM}: {message}", file=sys.stderr, flush=True)


def wasm_path(program):
    """The .wasm next to the program's JavaScript."""
    base, _ = os.path.splitext(program)
    return base + ".wasm"


def leb128(data, pos):
    value = shift = 0
    while True:
        byte = data[pos]
        pos += 1
        value |= (byte & 0x7F) << shift
        shift += 7
        if byte < 0x80:
            return value, pos


def asks_for_browser(program):
    """Whether the program's .wasm has a custom section `sdl3_runner` that
    holds `browser`."""
    try:
        with open(wasm_path(program), "rb") as f:
            data = f.read()
    except OSError:
        return False
    if data[:4] != b"\0asm":
        return False
    pos = 8
    try:
        while pos < len(data):
            section_id = data[pos]
            size, pos = leb128(data, pos + 1)
            end = pos + size
            if section_id == 0:
                name_len, name_pos = leb128(data, pos)
                name = data[name_pos:name_pos + name_len]
                if name == b"sdl3_runner" and b"browser" in data[name_pos + name_len:end]:
                    return True
            pos = end
    except IndexError:
        pass
    return False


def find_chrome():
    chrome = os.environ.get("CHROME")
    if chrome:
        path = shutil.which(chrome)
        if path is None:
            note(f"CHROME is {chrome!r}, which isn't a program")
        return path
    for name in ("google-chrome", "google-chrome-stable", "chromium", "chromium-browser", "chrome"):
        path = shutil.which(name)
        if path:
            return path
    for path in (
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
    ):
        if os.access(path, os.X_OK):
            return path
    return None


def run_node(program, args):
    node = os.environ.get("SDL3_EMSCRIPTEN_NODE") or "node"
    argv = [node, program, *args]
    sys.stdout.flush()
    sys.stderr.flush()
    if os.name == "posix":
        try:
            os.execvp(node, argv)
        except OSError as e:
            note(f"can't run {node}: {e}")
            return 127
    return subprocess.call(argv)


class Handler(http.server.SimpleHTTPRequestHandler):
    """The program's directory, and the page at SHELL_PATH."""

    extensions_map = {
        ".html": "text/html",
        ".js": "text/javascript",
        ".mjs": "text/javascript",
        ".wasm": "application/wasm",
        ".json": "application/json",
        ".map": "application/json",
        ".data": "application/octet-stream",
        "": "application/octet-stream",
    }

    def __init__(self, *args, shell, **kwargs):
        self.shell = shell
        super().__init__(*args, **kwargs)

    def translate_path(self, path):
        if path.split("?", 1)[0] == SHELL_PATH:
            return self.shell
        return super().translate_path(path)

    def end_headers(self):
        self.send_header("Cache-Control", "no-cache, no-store, must-revalidate")
        super().end_headers()

    def log_message(self, format, *args):  # noqa: A002 (the base class's name)
        if VERBOSE:
            note("http: " + format % args)


class Closed(Exception):
    """The browser closed the pipe (it exited)."""


class DevTools:
    """The DevTools protocol on Chromium's pipe: NUL-terminated JSON."""

    def __init__(self, read_fd, write_fd):
        self.read_fd = read_fd
        self.write_fd = write_fd
        self.next_id = 0
        self.messages = queue.Queue()
        self.events = []
        self.responses = {}
        threading.Thread(target=self._reader, daemon=True).start()

    def _reader(self):
        buffer = b""
        while True:
            try:
                chunk = os.read(self.read_fd, 1 << 16)
            except OSError:
                chunk = b""
            if not chunk:
                self.messages.put(None)
                return
            buffer += chunk
            *messages, buffer = buffer.split(b"\0")
            for message in messages:
                self.messages.put(json.loads(message))

    def _receive(self, timeout):
        try:
            message = self.messages.get(timeout=timeout)
        except queue.Empty:
            return False
        if message is None:
            self.messages.put(None)
            raise Closed()
        if "id" in message:
            self.responses[message["id"]] = message
        else:
            self.events.append(message)
        return True

    def send(self, method, params=None, session=None):
        self.next_id += 1
        message = {"id": self.next_id, "method": method, "params": params or {}}
        if session:
            message["sessionId"] = session
        data = json.dumps(message).encode() + b"\0"
        while data:
            data = data[os.write(self.write_fd, data):]
        return self.next_id

    def call(self, method, params=None, session=None, timeout=30):
        """Send a command and wait for its result (keeping the events that
        come in meanwhile)."""
        message_id = self.send(method, params, session)
        deadline = time.monotonic() + timeout
        while message_id not in self.responses:
            left = deadline - time.monotonic()
            if left <= 0:
                raise TimeoutError(f"no answer to {method}")
            self._receive(left)
        response = self.responses.pop(message_id)
        if "error" in response:
            raise RuntimeError(f"{method}: {response['error'].get('message')}")
        return response.get("result", {})

    def event(self, timeout):
        """The next event, or None after `timeout` seconds."""
        if not self.events:
            self._receive(timeout)
        return self.events.pop(0) if self.events else None


def high_fd(fd):
    """`fd` moved to 10 or above (close-on-exec), out of the way of the
    descriptors the browser gets (3 and 4)."""
    moved = fcntl.fcntl(fd, fcntl.F_DUPFD_CLOEXEC, 10)
    os.close(fd)
    return moved


class Browser:
    """A headless Chromium with its DevTools pipe."""

    def __init__(self, chrome, profile, log, no_sandbox):
        to_chrome_r, to_chrome_w = (high_fd(fd) for fd in os.pipe())
        from_chrome_r, from_chrome_w = (high_fd(fd) for fd in os.pipe())
        flags = CHROME_FLAGS + [f"--user-data-dir={profile}"]
        if no_sandbox:
            flags.append("--no-sandbox")
        flags += shlex.split(os.environ.get("SDL3_EMSCRIPTEN_CHROME_FLAGS", ""))
        argv = [chrome, *flags, "about:blank"]
        if VERBOSE:
            note("starting " + shlex.join(argv))
        log_fd = os.open(log, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
        try:
            self.pid = os.posix_spawn(
                chrome,
                argv,
                os.environ,
                file_actions=[
                    (os.POSIX_SPAWN_OPEN, 0, os.devnull, os.O_RDONLY, 0),
                    (os.POSIX_SPAWN_DUP2, log_fd, 1),
                    (os.POSIX_SPAWN_DUP2, log_fd, 2),
                    (os.POSIX_SPAWN_DUP2, to_chrome_r, 3),
                    (os.POSIX_SPAWN_DUP2, from_chrome_w, 4),
                ],
                setpgroup=0,
            )
        finally:
            os.close(log_fd)
            os.close(to_chrome_r)
            os.close(from_chrome_w)
        self.devtools = DevTools(from_chrome_r, to_chrome_w)
        self.fds = (from_chrome_r, to_chrome_w)
        self.status = None
        # (whether it answered on the pipe)
        self.started = False

    def exited(self):
        if self.status is None:
            pid, status = os.waitpid(self.pid, os.WNOHANG)
            if pid:
                self.status = status
        return self.status is not None

    def close(self):
        """Ask the browser to quit; kill it (its process group) if it doesn't."""
        try:
            self.devtools.send("Browser.close")
        except OSError:
            pass
        deadline = time.monotonic() + 5
        while not self.exited() and time.monotonic() < deadline:
            time.sleep(0.05)
        for sig in (signal.SIGTERM, signal.SIGKILL):
            if self.exited():
                break
            try:
                os.killpg(self.pid, sig)
            except ProcessLookupError:
                pass
            deadline = time.monotonic() + 5
            while not self.exited() and time.monotonic() < deadline:
                time.sleep(0.05)
        # (and whatever of its own it left in its process group)
        try:
            os.killpg(self.pid, signal.SIGKILL)
        except (ProcessLookupError, PermissionError):
            pass
        for fd in self.fds:
            os.close(fd)


def remote_text(obj):
    if "value" in obj:
        value = obj["value"]
        if isinstance(value, str):
            return value
        return json.dumps(value)
    return obj.get("unserializableValue") or obj.get("description") or obj.get("type", "")


def exception_text(details):
    exception = details.get("exception") or {}
    text = exception.get("description") or details.get("text", "exception")
    if details.get("url"):
        text += f"\n    at {details['url']}:{details.get('lineNumber', 0) + 1}"
    return text


def write(stream, text):
    stream.write(text + "\n")
    stream.flush()


def drive(browser, url, config, timeout):
    """Load the page and follow the program: its exit status."""
    devtools = browser.devtools
    devtools.call("Browser.getVersion")
    browser.started = True
    target = devtools.call("Target.createTarget", {"url": "about:blank"})["targetId"]
    session = devtools.call("Target.attachToTarget", {"targetId": target, "flatten": True})["sessionId"]
    devtools.call("Page.enable", {}, session)
    devtools.call("Runtime.addBinding", {"name": "sdl3RunnerReport"}, session)
    devtools.call(
        "Page.addScriptToEvaluateOnNewDocument",
        {"source": f"globalThis.sdl3RunnerConfig = {json.dumps(config)};"},
        session,
    )
    devtools.call("Runtime.enable", {}, session)
    devtools.call("Log.enable", {}, session)
    devtools.call("Inspector.enable", {}, session)
    if VERBOSE:
        note(f"loading {url}")
    navigation = devtools.call("Page.navigate", {"url": url}, session)
    if navigation.get("errorText"):
        note(f"can't load {url}: {navigation['errorText']}")
        return 1

    deadline = time.monotonic() + timeout if timeout else None
    aborted = None
    while True:
        if deadline is not None and time.monotonic() > deadline:
            note(f"timed out after {timeout} s")
            return 1
        if aborted is not None and time.monotonic() > aborted + 2:
            return 1
        event = devtools.event(0.25)
        if event is None:
            continue
        method = event.get("method")
        params = event.get("params", {})
        if event.get("sessionId") != session:
            if method == "Target.detachedFromTarget" and params.get("sessionId") == session:
                note("the page went away")
                return 1
            continue
        if method == "Runtime.consoleAPICalled":
            text = " ".join(remote_text(arg) for arg in params.get("args", []))
            kind = params.get("type")
            write(sys.stderr if kind in ("error", "warning", "assert") else sys.stdout, text)
        elif method == "Runtime.bindingCalled" and params.get("name") == "sdl3RunnerReport":
            what, value = json.loads(params["payload"])
            if what in ("exit", "drained"):
                return int(value) & 0xFF
            if what == "abort":
                aborted = time.monotonic()
        elif method == "Runtime.exceptionThrown":
            details = params.get("exceptionDetails", {})
            class_name = (details.get("exception") or {}).get("className")
            text = exception_text(details)
            # (emscripten's exit() throws an ExitStatus or 'unwind' to leave
            # the program's code: not an error)
            if class_name == "ExitStatus" or text in ("unwind", "Uncaught unwind"):
                continue
            write(sys.stderr, f"Uncaught {text}" if not text.startswith("Uncaught") else text)
            return 1
        elif method == "Log.entryAdded":
            entry = params.get("entry", {})
            if entry.get("level") == "error" or VERBOSE:
                where = f" ({entry['url']})" if entry.get("url") else ""
                write(sys.stderr, f"browser: {entry.get('text', '')}{where}")
        elif method == "Inspector.targetCrashed":
            note("the page crashed")
            return 1


def run_browser(chrome, program, args):
    directory = os.path.dirname(os.path.abspath(program))
    script = os.path.basename(program)
    try:
        timeout = float(os.environ.get("SDL3_EMSCRIPTEN_TIMEOUT") or 900)
    except ValueError:
        note("SDL3_EMSCRIPTEN_TIMEOUT isn't a number of seconds")
        return 1
    config = {"arguments": args, "env": dict(os.environ)}

    with tempfile.TemporaryDirectory(prefix="sdl3-emscripten-") as temp:
        shell = os.path.join(temp, "shell.html")
        with open(shell, "w", encoding="utf-8") as f:
            f.write(SHELL.format(title=script, script=script))
        handler = functools.partial(Handler, directory=directory, shell=shell)
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
        server.daemon_threads = True
        threading.Thread(target=server.serve_forever, daemon=True).start()
        url = f"http://127.0.0.1:{server.server_address[1]}{SHELL_PATH}"
        log = os.path.join(temp, "chrome.log")
        try:
            # As root Chromium's sandbox won't start; elsewhere it may not
            # either (containers without user namespaces): then start once
            # more without it.
            no_sandbox = hasattr(os, "geteuid") and os.geteuid() == 0
            while True:
                try:
                    browser = Browser(chrome, os.path.join(temp, "profile"), log, no_sandbox)
                except OSError as e:
                    note(f"can't start {chrome}: {e}")
                    return 1
                try:
                    return drive(browser, url, config, timeout)
                except Closed:
                    if not no_sandbox and not browser.started:
                        note("Chromium didn't start; once more with --no-sandbox")
                        no_sandbox = True
                        continue
                    note("Chromium exited (its log follows)")
                    show_log(log)
                    return 1
                except (TimeoutError, RuntimeError, OSError) as e:
                    note(f"DevTools: {e} (Chromium's log follows)")
                    show_log(log)
                    return 1
                finally:
                    browser.close()
                    if VERBOSE:
                        show_log(log)
        finally:
            server.shutdown()
            server.server_close()


def show_log(log):
    try:
        with open(log, encoding="utf-8", errors="replace") as f:
            sys.stderr.write(f.read())
        sys.stderr.flush()
    except OSError:
        pass


def main():
    if len(sys.argv) < 2:
        print(__doc__.split("\n\n", 2)[1], file=sys.stderr)
        return 2
    program, args = sys.argv[1], sys.argv[2:]
    mode = os.environ.get("SDL3_EMSCRIPTEN_BROWSER", "")
    if mode not in ("", "0", "1"):
        note(f"SDL3_EMSCRIPTEN_BROWSER is {mode!r}: 1 (browser), 0 (node) or unset")
        return 2
    if mode == "0" or (mode == "" and not asks_for_browser(program)):
        return run_node(program, args)
    chrome = find_chrome()
    if chrome is None:
        if mode == "1":
            note("no Chrome or Chromium found: set CHROME to one")
            return 1
        note(f"{os.path.basename(program)} asks for a browser, but there's no Chrome or Chromium (set CHROME): running it under node")
        return run_node(program, args)
    if os.name != "posix":
        note("the browser mode needs a POSIX system")
        return 1
    return run_browser(chrome, program, args)


if __name__ == "__main__":
    sys.exit(main())
