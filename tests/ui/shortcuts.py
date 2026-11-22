#!/usr/bin/env python3
"""Shortcut x state matrix for SSHL, driven with real key events.

Each case starts a fresh SSHL (the test-hook build), sets up a state (local tab idle / running a program, SSH tab,
dialog, prompt, panel, minimized window), presses keys and checks the result through the accessibility tree and the
process list. Keys go to the process with CGEventPostToPid, so the instance stays in the background and never takes
the keyboard from whoever is using the machine.

    cargo build -p sshl --release --features devhook
    python3 tests/ui/shortcuts.py                 # all cases
    python3 tests/ui/shortcuts.py ⌘Q 确认框        # cases whose name contains any of the words
    SSHL_UI_SSH=my-server python3 tests/ui/shortcuts.py   # also the SSH cases, with this saved connection

The terminal running it needs Accessibility permission (reading the tree, posting keys). The SSH cases connect for
real but only open and close the session.
"""
import json
import os
import signal
import socket
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
TOOLS = os.path.join(REPO, "target", "ui-tests")
APP = os.path.join(REPO, "target", "release", "sshl")
SOCK = "/tmp/sshl-ui-test.sock"
SSH_HOST = os.environ.get("SSHL_UI_SSH")
K = dict(w=13, q=12, t=17, n=45, k=40, comma=43, one=18, two=19, rbracket=30, i=34, o=31, p=35, d=2, m=46, f=3,
         b=11, tab=48, enter=36, esc=53, space=49, z=6, c=8)

pid = None
proc = None
# Sidebar entries right after launch: typing that leaks into the sidebar search filters them away.
sidebar = set()


def build_tools():
    os.makedirs(TOOLS, exist_ok=True)
    for name in ("keypost", "axstate", "axdump"):
        src, out = os.path.join(HERE, f"{name}.swift"), os.path.join(TOOLS, name)
        if not os.path.exists(out) or os.path.getmtime(out) < os.path.getmtime(src):
            subprocess.run(["swiftc", "-O", src, "-o", out], check=True)


def start():
    global pid, proc, sidebar
    stop()
    if os.path.exists(SOCK):
        os.remove(SOCK)
    env = dict(os.environ, SSHL_DEV_SOCKET=SOCK, SSHL_DEV_DISPLAY="secondary", SSHL_DEV_SIZE="1200x800")
    proc = subprocess.Popen([APP], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    pid = proc.pid
    for _ in range(100):
        if os.path.exists(SOCK):
            break
        time.sleep(0.1)
    time.sleep(1.2)
    # 窗口全被别的应用盖住时 GPUI 不重绘、无障碍树也停在旧的一帧：挪到底下露出一条（不激活、不抢焦点）
    subprocess.run(["osascript", "-e", f'tell application "System Events" to set position of window 1 of (first process whose unix id is {pid}) to {{360, 600}}'],
                   capture_output=True)
    time.sleep(0.4)
    sidebar = set(raw_state()["buttons"]) - {"", "新建连接", "设置", "收起侧边栏", "本机终端  ⌘T"}


def stop():
    global pid
    if pid and alive():
        os.kill(pid, signal.SIGTERM)
        time.sleep(0.5)
    subprocess.run(["pkill", "-f", APP], stderr=subprocess.DEVNULL)
    subprocess.run(["pkill", "-f", "sleep 70"], stderr=subprocess.DEVNULL)


def alive():
    # 自己起的子进程退出后是僵尸，kill -0 仍然成功：用 poll 收掉再看
    return proc is not None and proc.poll() is None


def dh(*cmds):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(SOCK)
    f = s.makefile("rw")
    for c in cmds:
        f.write(c + "\n")
        f.flush()
        f.readline()
    s.close()


def key(name, mods="none", wait=0.35):
    subprocess.run([f"{TOOLS}/keypost", str(pid), str(K[name]), mods])
    time.sleep(wait)


def raw_state():
    out = subprocess.run([f"{TOOLS}/axstate", str(pid)], capture_output=True, text=True).stdout
    return json.loads(out)


def state():
    d = raw_state()
    b = d["buttons"]
    settings = "还原默认" in b and "保存" in b
    return {
        "tabs": [t["name"] for t in d["tabs"]],
        "active": next((t["name"] for t in d["tabs"] if t["selected"]), None),
        "prompt": "取消" in b and not settings and "从 ~/.ssh/config 导入" not in b,
        "prompt_ok": [x for x in ("关闭", "退出", "断开") if x in b and not settings],
        "conn_dialog": "从 ~/.ssh/config 导入" in b,
        "settings": settings,
        "quick": "快速连接" in d["windows"],
        "snippets": "命令片段" in d["windows"],
        "favorites": "目录收藏" in d["windows"],
        "find": "关闭（Esc）" in b,
        "sidebar_ok": not sidebar or bool(sidebar & set(b)),
        "minimized": d["minimized"],
    }


def running(tag):
    return subprocess.run(["pgrep", "-f", f"sleep {tag}"], capture_output=True).returncode == 0


def local_tab(busy_tag=None):
    dh("keys cmd-t")
    time.sleep(1.4)
    if busy_tag:
        dh(f"type sleep {busy_tag}", "keys enter")
        time.sleep(0.6)


results = []


def case(name, fn):
    start()
    try:
        ok, detail = fn()
    except Exception as e:  # noqa: BLE001
        ok, detail = False, f"error: {e}"
    results.append((ok, name, detail))
    print(("PASS " if ok else "FAIL ") + name + (f"  — {detail}" if detail else ""), flush=True)


def expect(cond, detail):
    return (bool(cond), "" if cond else detail)


# ---------------------------------------------------------------- cases

def l_idle_cmd_w_esc():
    local_tab()
    key("w", "cmd")
    s1 = state()
    key("esc")
    s2 = state()
    return expect(s1["prompt"] and "关闭" in s1["prompt_ok"] and not s2["prompt"] and len(s2["tabs"]) == 1, f"{s1} / {s2}")


def l_idle_cmd_w_enter():
    local_tab()
    key("w", "cmd")
    key("enter", wait=0.8)
    s = state()
    return expect(not s["prompt"] and len(s["tabs"]) == 0, f"{s}")


def l_busy_cmd_w_enter():
    local_tab(7001)
    key("w", "cmd")
    s1 = state()
    key("enter", wait=0.8)
    s2 = state()
    return expect(s1["prompt"] and len(s2["tabs"]) == 0 and not running(7001), f"{s1} / {s2} sleep={running(7001)}")


def l_busy_cmd_w_x3_enter():
    local_tab(7002)
    for _ in range(3):
        key("w", "cmd", wait=0.15)
    key("enter", wait=0.8)
    s = state()
    return expect(not s["prompt"] and len(s["tabs"]) == 0, f"{s}")


def l_busy_cmd_w_twice_esc():
    local_tab(7003)
    key("w", "cmd")
    key("w", "cmd")
    key("esc")
    s = state()
    return expect(not s["prompt"] and len(s["tabs"]) == 1 and running(7003), f"{s} sleep={running(7003)}")


def l_busy_tab_tab_space():
    local_tab(7004)
    key("w", "cmd")
    key("tab")
    key("tab")
    key("space", wait=0.8)
    s = state()
    return expect(len(s["tabs"]) == 0 and not running(7004), f"{s}")


def l_busy_tab_enter_cancels():
    local_tab(7005)
    key("w", "cmd")
    key("tab")
    key("enter")
    s = state()
    return expect(not s["prompt"] and len(s["tabs"]) == 1 and running(7005), f"{s}")


def l_busy_space_closes():
    # the prompt opens with its default button selected: Space presses it
    local_tab(7008)
    key("w", "cmd")
    key("space", wait=0.8)
    s = state()
    return expect(len(s["tabs"]) == 0 and not running(7008), f"{s} sleep={running(7008)}")


def l_busy_shift_tab_enter_cancels():
    local_tab(7009)
    key("w", "cmd")
    key("tab", "shift")
    key("enter")
    s = state()
    return expect(not s["prompt"] and len(s["tabs"]) == 1 and running(7009), f"{s}")


def l_close_second_keyboard_reaches_first():
    local_tab(7006)
    local_tab(7007)
    key("w", "cmd")
    key("enter", wait=0.8)
    # focus must be back on the first tab's terminal: ctrl-c ends its sleep
    key("c", "ctrl", wait=0.6)
    s = state()
    return expect(len(s["tabs"]) == 1 and not running(7007) and not running(7006), f"{s} 7006={running(7006)} 7007={running(7007)}")


def q_cmd_q_esc():
    local_tab(7010)
    key("q", "cmd", wait=0.8)
    s1 = state()
    key("esc")
    return expect(alive() and s1["prompt"] and "退出" in s1["prompt_ok"] and running(7010) and not state()["prompt"], f"{s1}")


def q_cmd_q_enter():
    local_tab(7011)
    key("q", "cmd", wait=0.8)
    key("enter", wait=1.5)
    return expect(not alive() and not running(7011), f"alive={alive()}")


def q_cmd_q_x3_esc():
    local_tab(7012)
    for _ in range(3):
        key("q", "cmd", wait=0.15)
    time.sleep(0.6)
    key("esc", wait=0.6)
    s = state()
    return expect(alive() and not s["prompt"] and running(7012), f"{s}")


def q_minimized():
    local_tab(7013)
    key("m", "cmd", wait=1.2)
    s0 = state()
    key("q", "cmd", wait=1.5)
    s1 = state()
    key("esc")
    return expect(s0["minimized"] and alive() and s1["prompt"] and not s1["minimized"] and running(7013), f"{s0} / {s1}")


def q_no_tabs():
    key("q", "cmd", wait=1.5)
    return expect(not alive(), f"alive={alive()}")


def q_while_close_prompt():
    local_tab(7014)
    key("w", "cmd")
    key("q", "cmd", wait=0.8)
    s1 = state()
    key("esc")
    s2 = state()
    return expect(alive() and s1["prompt_ok"] == ["关闭"] and not s2["prompt"] and running(7014), f"{s1} / {s2}")


def m_prompt_blocks(shortcut, mods, check):
    def run():
        local_tab(7020)
        local_tab(7021)
        before = state()
        key("w", "cmd")
        key(shortcut, mods, wait=0.8)
        s = state()
        bad = check(before, s)
        key("esc")
        after = state()
        return expect(s["prompt"] and not bad and not after["prompt"] and len(after["tabs"]) == 2,
                      f"bad={bad} {s} / after={after}")
    return run


def d_conn_dialog_blocks_cmd_w():
    local_tab(7030)
    key("n", "cmd", wait=0.8)
    key("w", "cmd", wait=0.6)
    s = state()
    key("esc")
    after = state()
    return expect(s["conn_dialog"] and not s["prompt"] and len(s["tabs"]) == 1 and not after["conn_dialog"], f"{s} / {after}")


def d_conn_dialog_tab_no_leak():
    local_tab(7031)
    key("n", "cmd", wait=0.8)
    for _ in range(12):
        key("tab", wait=0.1)
    key("z", wait=0.4)
    s = state()
    key("esc")
    return expect(s["conn_dialog"] and s["sidebar_ok"], f"{s}")


def d_settings_blocks_cmd_w():
    local_tab(7032)
    key("comma", "cmd", wait=0.8)
    key("w", "cmd", wait=0.6)
    s = state()
    key("esc")
    after = state()
    return expect(s["settings"] and len(s["tabs"]) == 1 and not after["settings"], f"{s} / {after}")


def p_panel_tab(shortcut, mods, flag):
    def run():
        local_tab()
        key(shortcut, mods, wait=0.8)
        key("tab", wait=0.2)
        key("tab", "shift", wait=0.2)
        key("z", wait=0.4)
        s = state()
        key("esc")
        after = state()
        return expect(s[flag] and s["sidebar_ok"] and not after[flag], f"{s} / {after}")
    return run


def f_find_tab():
    local_tab()
    key("f", "cmd", wait=0.6)
    key("tab", wait=0.2)
    key("z", wait=0.4)
    s = state()
    key("esc")
    after = state()
    return expect(s["find"] and s["sidebar_ok"] and not after["find"], f"{s} / {after}")


def r_connect():
    name = SSH_HOST
    out = subprocess.run([f"{TOOLS}/axdump", str(pid)], capture_output=True, text=True).stdout
    line = next(l for l in out.splitlines() if f"AXButton | {name} |" in l)
    x, y = line.split()[-2:]
    dh(f"click {x} {y} 2")
    for _ in range(40):
        time.sleep(0.25)
        if name in state()["tabs"]:
            time.sleep(1.5)
            return True
    return False


def r_ssh_cmd_w():
    if not r_connect():
        return False, "could not connect"
    key("w", "cmd")
    s1 = state()
    key("esc")
    s2 = state()
    key("w", "cmd")
    key("enter", wait=1.0)
    s3 = state()
    return expect(s1["prompt"] and "断开" in s1["prompt_ok"] and len(s2["tabs"]) == 1 and len(s3["tabs"]) == 0, f"{s1} / {s2} / {s3}")


def r_disconnected_cmd_w():
    if not r_connect():
        return False, "could not connect"
    dh("type exit", "keys enter")
    time.sleep(2.0)
    key("w", "cmd", wait=0.8)
    s = state()
    return expect(not s["prompt"] and len(s["tabs"]) == 0, f"{s}")


CASES = [
    ("本机·空闲 ⌘W → 弹框，Esc 不关", l_idle_cmd_w_esc),
    ("本机·空闲 ⌘W ↵ → 关", l_idle_cmd_w_enter),
    ("本机·跑程序 ⌘W ↵ → 关，程序结束", l_busy_cmd_w_enter),
    ("本机·跑程序 ⌘W×3 ↵ → 不叠框，一次关掉", l_busy_cmd_w_x3_enter),
    ("本机·跑程序 ⌘W ⌘W Esc → 不关，程序在", l_busy_cmd_w_twice_esc),
    ("确认框 Tab Tab 空格 → 关", l_busy_tab_tab_space),
    ("确认框 Tab ↵（=取消）→ 不关", l_busy_tab_enter_cancels),
    ("确认框 空格（默认按钮已选中）→ 关", l_busy_space_closes),
    ("确认框 ⇧Tab ↵（=取消）→ 不关", l_busy_shift_tab_enter_cancels),
    ("关掉第二个标签后键盘回到第一个", l_close_second_keyboard_reaches_first),
    ("⌘Q → 弹框，Esc 不退", q_cmd_q_esc),
    ("⌘Q ↵ → 退出", q_cmd_q_enter),
    ("⌘Q×3 Esc → 只一个框，不退", q_cmd_q_x3_esc),
    ("⌘M 最小化后 ⌘Q → 窗口拿出来并弹框", q_minimized),
    ("没有标签 ⌘Q → 直接退", q_no_tabs),
    ("关标签确认框开着时 ⌘Q → 不叠退出框", q_while_close_prompt),
    ("确认框开着 ⌘N → 不开新建连接",
     m_prompt_blocks("n", "cmd", lambda b, s: s["conn_dialog"])),
    ("确认框开着 ⌘T → 不开新标签",
     m_prompt_blocks("t", "cmd", lambda b, s: len(s["tabs"]) != len(b["tabs"]))),
    ("确认框开着 ⌘K → 不开快速连接",
     m_prompt_blocks("k", "cmd", lambda b, s: s["quick"])),
    ("确认框开着 ⌘, → 不开设置",
     m_prompt_blocks("comma", "cmd", lambda b, s: s["settings"])),
    ("确认框开着 ⌘⇧P → 不开命令片段",
     m_prompt_blocks("p", "cmd+shift", lambda b, s: s["snippets"])),
    ("确认框开着 ⌘⇧D → 不开目录收藏",
     m_prompt_blocks("d", "cmd+shift", lambda b, s: s["favorites"])),
    ("确认框开着 ⌘1 → 不切标签",
     m_prompt_blocks("one", "cmd", lambda b, s: s["active"] != b["active"])),
    ("确认框开着 ⌘⇧] → 不切标签",
     m_prompt_blocks("rbracket", "cmd+shift", lambda b, s: s["active"] != b["active"])),
    ("新建连接开着 ⌘W → 不关标签", d_conn_dialog_blocks_cmd_w),
    ("新建连接 Tab×12 打字 → 不漏到侧栏", d_conn_dialog_tab_no_leak),
    ("设置开着 ⌘W → 不关标签", d_settings_blocks_cmd_w),
    ("⌘K 面板 Tab ⇧Tab 打字 → 不漏，Esc 关", p_panel_tab("k", "cmd", "quick")),
    ("命令片段面板 Tab ⇧Tab 打字 → 不漏，Esc 关", p_panel_tab("p", "cmd+shift", "snippets")),
    ("目录收藏面板 Tab ⇧Tab 打字 → 不漏，Esc 关", p_panel_tab("d", "cmd+shift", "favorites")),
    ("⌘F 搜索条 Tab 打字 → 不漏，Esc 关", f_find_tab),
    ("SSH 已连接 ⌘W → 断开确认，Esc 不关，↵ 关", r_ssh_cmd_w),
    ("SSH 已断开 ⌘W → 直接关（里面没有进程）", r_disconnected_cmd_w),
]

build_tools()
only = sys.argv[1:]
for name, fn in CASES:
    if only and not any(o in name for o in only):
        continue
    if name.startswith("SSH") and not SSH_HOST:
        print("SKIP " + name + "  — set SSHL_UI_SSH to a saved connection to run it")
        continue
    case(name, fn)
stop()
print(f"\n{sum(ok for ok, _, _ in results)}/{len(results)} passed")
