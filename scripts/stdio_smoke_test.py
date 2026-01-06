#!/usr/bin/env python3
import json
import os
import select
import subprocess
import sys
import time


def write_msg(proc, msg):
    payload = json.dumps(msg, separators=(",", ":")).encode("utf-8")
    header = f"Content-Length: {len(payload)}\r\n\r\n".encode("utf-8")
    proc.stdin.write(header + payload)
    proc.stdin.flush()


def read_message(proc, timeout_s, stdout_buf, stderr_lines):
    deadline = time.time() + timeout_s
    stdout_fd = proc.stdout.fileno()
    stderr_fd = proc.stderr.fileno()

    def read_from_fd(fd):
        try:
            return os.read(fd, 4096)
        except OSError:
            return b""

    while time.time() < deadline:
        if proc.poll() is not None:
            return None
        rlist, _, _ = select.select([stdout_fd, stderr_fd], [], [], 0.2)
        for fd in rlist:
            if fd == stderr_fd:
                chunk = read_from_fd(fd)
                if chunk:
                    stderr_lines.append(chunk.decode("utf-8", errors="replace").rstrip())
                continue
            chunk = read_from_fd(fd)
            if chunk:
                stdout_buf.extend(chunk)

        header_end = stdout_buf.find(b"\r\n\r\n")
        if header_end == -1:
            continue

        header_bytes = stdout_buf[:header_end].decode("utf-8", errors="replace")
        stdout_buf[: header_end + 4] = b""

        content_length = None
        for line in header_bytes.splitlines():
            if line.lower().startswith("content-length:"):
                try:
                    content_length = int(line.split(":", 1)[1].strip())
                except ValueError:
                    content_length = None
                break
        if content_length is None:
            stderr_lines.append(f"invalid headers: {header_bytes}")
            continue

        while len(stdout_buf) < content_length and time.time() < deadline:
            rlist, _, _ = select.select([stdout_fd, stderr_fd], [], [], 0.2)
            for fd in rlist:
                if fd == stderr_fd:
                    chunk = read_from_fd(fd)
                    if chunk:
                        stderr_lines.append(
                            chunk.decode("utf-8", errors="replace").rstrip()
                        )
                    continue
                chunk = read_from_fd(fd)
                if chunk:
                    stdout_buf.extend(chunk)

        if len(stdout_buf) < content_length:
            return None

        body = bytes(stdout_buf[:content_length])
        del stdout_buf[:content_length]
        try:
            return json.loads(body.decode("utf-8"))
        except Exception:
            stderr_lines.append(f"non-json message body: {body!r}")
            return None
    return None


def main():
    env = os.environ.copy()
    env.setdefault("RUST_LOG", "info")
    cmd = ["cargo", "run", "-q"]
    proc = subprocess.Popen(
        cmd,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=False,
        bufsize=0,
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

        stdout_buf = bytearray()
        init_resp = read_message(proc, 15, stdout_buf, stderr_lines)
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
        tools_resp = read_message(proc, 15, stdout_buf, stderr_lines)
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
