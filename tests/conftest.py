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
            occurrences = sum(urlsplit(path).path == urlsplit(self.path).path for path, _ in requests)
            status = int(mode[6:]) if mode in ('status401', 'status403', 'status404', 'status429', 'status503') else None
            if mode == 'transient' and occurrences == 1:
                status = 503
            if status:
                self.send_response(status)
                self.send_header('Content-Length', '0')
                self.end_headers()
                return
            if mode in ('redirect', 'youtube_challenge'):
                self.send_response(302)
                self.send_header('Location', 'https://www.google.com/sorry/index?continue=private'
                                 if mode == 'youtube_challenge' else 'http://127.0.0.1/secret')
                self.end_headers()
                return
            body = (b'RIFF' + b'\x10\x00\x00\x00WEBPfixture' if mode == 'image' else
                    b'<html>error</html>' if mode == 'html' else media_bodies.get(mode, BODY))
            range_header = self.headers.get('Range')
            chunked = mode in ('range_chunked', 'range_chunked_truncated')
            if chunked:
                self.protocol_version = 'HTTP/1.1'
            if mode == 'bad_range' or range_header and mode in (
                    'range', 'interrupted', 'transient', 'range_chunked', 'range_chunked_truncated', 'range_zero_length'):
                import re
                start, end = re.fullmatch(r'bytes=(\d+)-(\d*)', range_header or 'bytes=0-').groups()
                start, end = int(start), min(int(end) if end else len(body)-1, len(body)-1)
                size = len(body)
                body = body[start:end+1]
                self.send_response(206)
                self.send_header('Content-Range', f'bytes {start + (1 if mode == "bad_range" else 0)}-{end}/{size}')
            else:
                self.send_response(200)
            self.send_header('Content-Type','video/mp4')
            if chunked:
                self.send_header('Transfer-Encoding', 'chunked')
            elif mode != 'unknown':
                length = str(len(body) + (1000 if mode in ('truncated','stall') else 0))
                self.send_header('Content-Length', ('000' if mode == 'range_zero_length' else '') + length)
            self.end_headers()
            try:
                if chunked:
                    payload = body[:1024] if mode == 'range_chunked_truncated' else body
                    self.wfile.write(f'{len(payload):X}\r\n'.encode() + payload + b'\r\n0\r\n\r\n')
                    return
                if mode == 'interrupted' and occurrences == 1:
                    self.wfile.write(body[:1024]); self.wfile.flush()
                    return
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
