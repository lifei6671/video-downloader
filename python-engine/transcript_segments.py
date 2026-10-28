"""Private, versioned VAD-window artifact; never emitted as bridge text."""
import json
import re

RATE = 16000
FILENAME = 'transcript-segments.json'
MAX_BYTES = 16 * 1024 * 1024


def parse_srt(text, frames):
    rows = []
    for index, block in enumerate(re.split(r'\n\s*\n', text.strip()), 1):
        if not block:
            continue
        lines = block.splitlines()
        if len(lines) < 3 or lines[0] != str(index):
            raise ValueError('Invalid SRT cue')
        match = re.fullmatch(r'(\d+):(\d{2}):(\d{2}),(\d{3}) --> (\d+):(\d{2}):(\d{2}),(\d{3})', lines[1])
        if not match:
            raise ValueError('Invalid SRT time')
        values = list(map(int, match.groups()))
        if any(values[i] >= 60 for i in (1, 2, 5, 6)):
            raise ValueError('Invalid SRT clock')
        def milliseconds(v):
            return ((v[0] * 60 + v[1]) * 60 + v[2]) * 1000 + v[3]
        rows.append(dict(start_ms=milliseconds(values[:4]), end_ms=milliseconds(values[4:]), text='\n'.join(lines[2:]).strip()))
    validate(rows, frames)
    return rows


def validate(rows, frames):
    if type(frames) is not int or frames <= 0 or len(rows) > 100000:
        raise ValueError('Invalid audio duration or segment count')
    previous = 0
    for row in rows:
        a, b, text = row['start_ms'], row['end_ms'], row['text']
        if (type(a) is not int or type(b) is not int or a < previous or b <= a or b * 16 > frames
                or not isinstance(text, str) or not text.strip() or text == '/sil'):
            raise ValueError('Invalid timed segment')
        previous = b


def write(stage, rows, frames, vad_revision):
    validate(rows, frames)
    value = dict(schema_version=1, time_base='media_zero', timing_source='fsmn_vad', precision='vad_segment',
                 sample_rate=RATE, frames=frames, vad_revision=vad_revision, segments=rows)
    body = json.dumps(value, ensure_ascii=False, separators=(',', ':')).encode('utf-8')
    if len(body) > MAX_BYTES:
        raise ValueError('Timing artifact too large')
    (stage / FILENAME).write_bytes(body)
    return dict(filename=FILENAME, bytes=len(body))
