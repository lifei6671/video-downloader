import http.server
import threading
from urllib.parse import urlsplit

import pytest

BODY = b'\x00\x00\x00\x18ftypmp42' + b'payload' * 600


@pytest.fixture(autouse=True)
def no_real_browser_credentials(monkeypatch):
    import engine
    monkeypatch.setattr(engine, 'load_default_cookies', lambda *_: None)


@pytest.fixture
def media_bodies():
    return {}


@pytest.fixture
def media_proxy(media_bodies):
    requests = []
    started = threading.Event()
    released = threading.Event()
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass
        def do_GET(self):
            requests.append((self.path, dict(self.headers)))
            mode = urlsplit(self.path).path.split('/')[-1].split('.')[0]
            if mode == 'redirect':
                self.send_response(302)
                self.send_header('Location','http://127.0.0.1/secret')
                self.end_headers()
                return
            body = (b'RIFF' + b'\x10\x00\x00\x00WEBPfixture' if mode == 'image' else
                    b'<html>error</html>' if mode == 'html' else media_bodies.get(mode, BODY))
            self.send_response(200)
            self.send_header('Content-Type','video/mp4')
            if mode != 'unknown':
                self.send_header('Content-Length',str(len(body) + (1000 if mode in ('truncated','stall') else 0)))
            self.end_headers()
            try:
                if mode == 'stall':
                    self.wfile.write(body[:1024]); self.wfile.flush()
                    started.set(); released.wait(15)
                    return
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
                pass
    server = http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
    thread = threading.Thread(target=server.serve_forever,daemon=True)
    thread.start()
    yield f'http://127.0.0.1:{server.server_port}', requests, started, released
    released.set()
    server.shutdown(); server.server_close(); thread.join()
