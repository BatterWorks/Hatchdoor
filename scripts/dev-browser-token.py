#!/usr/bin/env python3
"""Hand the dev web token to a test browser.

A live check in a real browser has to sign in, and the web token must not pass
through a tool call or a transcript to get there. This serves the token on
127.0.0.1 to the dev frontend's pages only, so the page fetches it and stores
it itself. `just dev-browser-token` starts it and prints the line to run in
the page.

The token is found the way the backend finds it: the environment first, then
`.env`. It stops by itself after LIFETIME_SECONDS, so a forgotten helper does
not keep answering.
"""

import argparse
import http.server
import os
import sys
import threading

TOKEN_SETTING = "HATCHDOOR_WEB_BEARER_TOKEN"
ENV_FILE = ".env"
LIFETIME_SECONDS = 900


def read_token():
    exported = os.environ.get(TOKEN_SETTING, "").strip()
    if exported:
        return exported
    try:
        with open(ENV_FILE, encoding="utf-8") as lines:
            for line in lines:
                name, _, value = line.strip().partition("=")
                if name == TOKEN_SETTING:
                    return value.strip().strip("\"'")
    except OSError as error:
        sys.exit(f"cannot read {ENV_FILE}: {error}")
    return ""


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument(
        "--origin",
        action="append",
        required=True,
        help="a page origin allowed to read the token; repeat for several",
    )
    args = parser.parse_args()

    token = read_token()
    if not token:
        sys.exit(f"{TOKEN_SETTING} is set neither in the environment nor in {ENV_FILE}")

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            # The page's cross-port fetch always names its origin. A request
            # that names another one, or none (a same-origin fetch from a page
            # that reached this port by another name, a script tag), gets
            # nothing.
            origin = self.headers.get("Origin")
            if origin not in args.origin:
                self.send_error(403)
                return
            body = token.encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/plain; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.send_header("X-Content-Type-Options", "nosniff")
            self.send_header("Access-Control-Allow-Origin", origin)
            self.send_header("Vary", "Origin")
            self.end_headers()
            self.wfile.write(body)

    server = http.server.ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    threading.Timer(LIFETIME_SECONDS, server.shutdown).start()
    print(
        f"serving the web token on http://127.0.0.1:{args.port}/ "
        f"for {LIFETIME_SECONDS}s",
        flush=True,
    )
    server.serve_forever()
    print("lifetime reached, stopped", flush=True)


if __name__ == "__main__":
    main()
