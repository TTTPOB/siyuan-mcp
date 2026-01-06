#!/usr/bin/env python3
import json
import os
import select
import subprocess
import sys
import time


def write_msg(proc, msg):
    payload = json.dumps(msg, separators=(",", ":"))
    proc.stdin.write(payload + "\n")
    proc.stdin.flush()


def read_message(proc, timeout_s, stderr_lines):
    deadline = time.time() + timeout_s
    stdout_fd = proc.stdout.fileno()
    stderr_fd = proc.stderr.fileno()

    while time.time() < deadline:
        if proc.poll() is not None:
            return None
        rlist, _, _ = select.select([stdout_fd, stderr_fd], [], [], 0.2)
        for fd in rlist:
            if fd == stderr_fd:
                line = proc.stderr.readline()
                if line:
                    stderr_lines.append(line.rstrip())
                continue
            line = proc.stdout.readline()
            if not line:
                continue
            line = line.strip()
            if not line:
                continue
            try:
                return json.loads(line)
            except Exception:
                stderr_lines.append(f"non-json stdout: {line}")
                continue
    return None


def main():
    env = os.environ.copy()
    env.setdefault("RUST_LOG", "info")
    timeout_s = float(env.get("STDIO_SMOKE_TIMEOUT_S", "60"))
    cmd = ["cargo", "run", "-q"]
    proc = subprocess.Popen(
        cmd,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
        env=env,
    )

    stderr_lines = []
    try:
        init_req = {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {
                    "tools": {"listChanged": True},
                    "resources": {"listChanged": True},
                },
                "clientInfo": {"name": "stdio-smoke-test", "version": "0.1.0"},
            },
        }
        write_msg(proc, init_req)

        init_resp = read_message(proc, timeout_s, stderr_lines)
        if not init_resp:
            print("initialize: no response", file=sys.stderr)
            if stderr_lines:
                print("stderr tail:", file=sys.stderr)
                for line in stderr_lines[-20:]:
                    print(line, file=sys.stderr)
            return 1
        if init_resp.get("id") != 1:
            print("initialize: unexpected response", init_resp, file=sys.stderr)
            return 1

        print("initialize response:", init_resp)

        write_msg(proc, {"jsonrpc": "2.0", "method": "notifications/initialized"})

        tools_req = {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}
        write_msg(proc, tools_req)
        tools_resp = read_message(proc, timeout_s, stderr_lines)
        if not tools_resp:
            print("tools/list: no response", file=sys.stderr)
            if stderr_lines:
                print("stderr tail:", file=sys.stderr)
                for line in stderr_lines[-20:]:
                    print(line, file=sys.stderr)
            return 1
        if tools_resp.get("id") != 2:
            print("tools/list: unexpected response", tools_resp, file=sys.stderr)
            return 1

        print("tools/list response:", tools_resp)
        return 0
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()


if __name__ == "__main__":
    raise SystemExit(main())
