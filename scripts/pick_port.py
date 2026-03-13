#!/usr/bin/env python3
import os
import socket


def free_port(host):
    s = socket.socket()
    s.bind((host, 0))
    port = s.getsockname()[1]
    s.close()
    return port


def port_is_free(host, port):
    s = socket.socket()
    s.settimeout(0.1)
    result = s.connect_ex((host, port))
    s.close()
    return result != 0


def main():
    host = os.environ.get("MUON_CACHE_HOST", "127.0.0.1")
    port_env = os.environ.get("MUON_CACHE_PORT", "")
    port = 0
    try:
        if port_env:
            port = int(port_env)
    except ValueError:
        port = 0

    if port and port_is_free(host, port):
        print(port)
        return

    print(free_port(host))


if __name__ == "__main__":
    main()
