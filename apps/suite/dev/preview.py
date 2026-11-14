#!/usr/bin/env python3
"""Dev-only: serves apps/suite/ui in a normal browser WITH a mocked backend.
   python3 apps/suite/dev/preview.py  ->  http://127.0.0.1:8765/?mock=virgin|locked|ready|none
Never part of the app bundle (the app embeds only apps/suite/ui)."""
import http.server, pathlib, functools

HERE = pathlib.Path(__file__).resolve().parent
UI = HERE.parent / "ui"

class H(http.server.SimpleHTTPRequestHandler):
    def do_GET(self):
        path = self.path.split("?")[0]
        if path == "/dev/mock.js":
            return self._send((HERE / "mock.js").read_bytes(), "text/javascript")
        if path in ("/", "/index.html"):
            html = (UI / "index.html").read_text().replace(
                '<script src="app.js">', '<script src="dev/mock.js"></script>\n  <script src="app.js">')
            return self._send(html.encode(), "text/html; charset=utf-8")
        return super().do_GET()

    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def _send(self, body, ctype):
        self.send_response(200); self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)

if __name__ == "__main__":
    handler = functools.partial(H, directory=str(UI))
    print("preview: http://127.0.0.1:8765/?mock=virgin  (Ctrl+C para sair)")
    http.server.ThreadingHTTPServer(("127.0.0.1", 8765), handler).serve_forever()
