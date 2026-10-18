"""Independent loopback-only tdl HTTP fixture. No Telegram imports, credentials or network."""
import http.server
import json
import os
from pathlib import Path
import sys
import time
from urllib.parse import urlparse

root = Path(__file__).parent
options = json.loads((root / 'options.json').read_text(encoding='utf-8'))


def event(kind, **fields):
    with (root / 'events.jsonl').open('a', encoding='utf-8') as output:
        output.write(json.dumps(dict(kind=kind, pid=os.getpid(), **fields)) + '\n')


if 'version' in sys.argv:
    # Version probes must override storage, avoiding tdl's migration/root hooks on user data.
    assert '--storage' in sys.argv
    event('version', isolated=True)
    print(options.get('version_output', 'Version: ' + options.get('version', '0.20.4')))
    sys.exit(options.get('version_exit', 0))
elif sys.argv[1:] == ['env', '-json', 'GOBIN', 'GOPATH']:
    event('go_env', toolchain=os.environ.get('GOTOOLCHAIN'))
    print(json.dumps(options.get('go_env', {})))
elif '--help' in sys.argv:
    event('help')
    print(options.get('help', '--serve --port --url --type qr'))
elif 'login' in sys.argv:
    storage = sys.argv[sys.argv.index('--storage') + 1]
    import csv
    fields = dict(item.split('=', 1) for item in next(csv.reader([storage])))
    assert fields['type'] == 'bolt'
    target = Path(fields['path'])
    assert not target.exists(), 'must never overwrite existing authentication data'
    target.mkdir()
    (target / 'mock-session').write_text('mock-authentication')
    child = None
    if options.get('login_child'):
        import subprocess
        child = subprocess.Popen([sys.executable, '-c', 'import time;time.sleep(120)'], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    event('login', storage=storage, namespace=sys.argv[sys.argv.index('--ns') + 1], mode=sys.argv[sys.argv.index('--type') + 1], child_pid=child.pid if child else None)
    if options.get('login_ui'):
        if os.name == 'nt':
            import ctypes
            handle = ctypes.windll.kernel32.GetStdHandle(-10)
            mode = ctypes.c_uint()
            assert ctypes.windll.kernel32.GetConsoleMode(handle, ctypes.byref(mode))
            assert ctypes.windll.kernel32.SetConsoleMode(handle, mode.value & ~7)
        else:
            import tty
            tty.setraw(sys.stdin.fileno())
        print('Scan QR code with your Telegram app...\r')
        n = 29
        bits = [[4 <= x < n-4 and 4 <= y < n-4 and (x+y) % 3 == 0 for x in range(n)] for y in range(n)]
        text = ''
        for y in range(0, n, 2):
            for x in range(n):
                a,b = bits[y][x], bits[y+1][x] if y+1 < n else False
                text += (' ' if a else '█') if a == b else ('▄' if a else '▀')
            text += '\r\n'
        sys.stdout.write(text + '\x1b[15A')
        sys.stdout.flush()
        time.sleep(0.3)
        sys.stdout.write('\x1b[2J\x1b[H? Enter 2FA Password:\x1b[6n')
        sys.stdout.flush()
        response = b''
        while not response.endswith(b'R'):
            response += os.read(sys.stdin.fileno(), 1)
        assert response.startswith(b'\x1b[')
        password = bytearray()
        while True:
            char = os.read(sys.stdin.fileno(), 1)
            if char in (b'\r', b'\n'): break
            password.extend(char)
        accepted = password.decode('utf-8') == options.get('password', 'test-only-password')
        password[:] = b'\0' * len(password)
        event('password', accepted=accepted)
        if not accepted:
            print('\r\nPASSWORD_HASH_INVALID', flush=True)
            sys.exit(1)
    time.sleep(options.get('login_delay', 0))
    sys.exit(options.get('login_exit', 0))
elif 'sleep' in sys.argv:
    event('sleep')
    time.sleep(120)
elif 'exit' in sys.argv:
    pass
else:
    assert sys.argv[1:3] == ['dl', '-u'] and '--serve' in sys.argv
    port = int(sys.argv[sys.argv.index('--port') + 1])
    url = sys.argv[sys.argv.index('-u') + 1]
    event('session', storage=sys.argv[sys.argv.index('--storage') + 1] if '--storage' in sys.argv else None, namespace=sys.argv[sys.argv.index('--ns') + 1] if '--ns' in sys.argv else None)
    parts = urlparse(url).path.strip('/').split('/')
    channel = parts[1] if parts[0] == 'c' else str(options.get('channel', 100))
    message = parts[-1]

    class Handler(http.server.BaseHTTPRequestHandler):
        protocol_version = 'HTTP/1.1'

        def log_message(self, *_):
            pass

        def do_HEAD(self):
            self.respond(True)

        def do_GET(self):
            self.respond(False)

        def respond(self, head):
            event('request', method=self.command, path=self.path)
            if self.path == '/':
                content = options.get('html', f'<title>tdl serve(beta)</title><a href="{channel}/{message}">file</a>').encode()
                self.send_response(options.get('home_status', 200))
                if options.get('home_status') == 302:
                    self.send_header('Location', 'http://example.invalid/forbidden')
                self.send_header('Content-Length', str(len(content)))
                self.end_headers()
                self.wfile.write(content)
                return
            parts = self.path.strip('/').split('/')
            if len(parts) != 2 or any(not p.isdigit() or p.startswith('0') for p in parts):
                self.send_error(404)
                return
            item = {**options, **options.get('files', {}).get(self.path, {})}
            size = item.get('size', 262144)
            body = b'\x00\x00\x00\x18ftypisom' + bytes([item.get('fill', 118)]) * (size - 12)
            if item.get('bad_header'):
                body = b'<html>error!</html>' + body[18:]
            time.sleep(item.get('head_delay', 0) if head else item.get('get_delay', 0))
            status = item.get('head_status' if head else 'get_status', 200)
            self.send_response(status)
            self.send_header('Content-Length', str(item.get('declared_size', size)))
            # Emit UTF-8 verbatim, as the upstream Go server does for a Unicode filename.
            self._headers_buffer.append(('Content-Type: ' + item.get('mime', 'video/mp4') + '\r\n').encode())
            self._headers_buffer.append(('Content-Disposition: attachment; filename="' + item.get('name', '原始视频.mp4') + '"\r\n').encode())
            if item.get('encoding'):
                self.send_header('Content-Encoding', options['encoding'])
            if item.get('range') and not head:
                self.send_header('Content-Range', options['range'])
            self.send_header('Connection', 'close')
            self.end_headers()
            if not head:
                try:
                    content = body[:item.get('actual_size', len(body))]
                    chunk = item.get('chunk', 16384)
                    for offset in range(0, len(content), chunk):
                        self.wfile.write(content[offset:offset + chunk])
                        self.wfile.flush()
                        time.sleep(item.get('chunk_delay', 0))
                except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
                    pass
            self.close_connection = True

    time.sleep(options.get('startup_delay', 0))
    server = http.server.ThreadingHTTPServer(('127.0.0.1', port), Handler)
    event('serve', port=port, channel=channel, message=message)
    server.serve_forever()
