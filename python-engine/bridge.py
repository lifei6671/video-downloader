"""Protocol v3: one operation per process; EOF cancels even blocked network I/O."""
import json
import os
import sys
import threading

from engine import ENGINE_VERSION, YTDLP_VERSION, EngineError, PROTOCOL_VERSION, dtk_adapter, check_version, operate

MAX_LINE = 1024 * 1024


def emit(value):
    sys.stdout.buffer.write(json.dumps(value, ensure_ascii=False, allow_nan=False).encode('utf-8') + b'\n')
    sys.stdout.buffer.flush()


def watch_input():
    # After the single request, any more input is a protocol violation. EOF means
    # the owning process is gone/cancelled. os._exit interrupts synchronous yt-dlp.
    sys.stdin.buffer.read(1)
    os._exit(3)


def main():
    request_id = None
    try:
        check_version()
        emit(dict(protocol_version=PROTOCOL_VERSION, type='ready', engine_version=ENGINE_VERSION,
                  yt_dlp_version=YTDLP_VERSION, dtk_version=dtk_adapter.VERSION, dtk_commit=dtk_adapter.COMMIT))
        line = sys.stdin.buffer.readline(MAX_LINE + 1)
        if not line:
            return 3
        if len(line) > MAX_LINE or not line.endswith(b'\n'):
            raise EngineError('ENGINE_PROTOCOL_ERROR', 'protocol')
        request = json.loads(line)
        if (not isinstance(request, dict) or request.get('protocol_version') != PROTOCOL_VERSION
                or request.get('method') not in ('identify_input', 'resolve_video', 'download_video', 'check_dependencies', 'install_dependency', 'extract_cover', 'transcribe_video', 'probe_local_video', 'extract_keyframes', 'get_audio_excerpt')
                or not isinstance(request.get('id'), str) or not 0 < len(request['id']) <= 128):
            raise EngineError('ENGINE_PROTOCOL_ERROR', 'protocol')
        request_id = request['id']
        threading.Thread(target=watch_input, daemon=True).start()
        def event(event_type, **fields):
            emit(dict(protocol_version=PROTOCOL_VERSION, id=request_id, type=event_type, **fields))
        operate(request['method'], request.get('params'), event)
        # Avoid Python finalization waiting on the daemon's blocked stdin buffer lock.
        return 0
    except EngineError as error:
        code, stage = error.code, error.stage
    except (ValueError, UnicodeError, TypeError):
        code, stage = 'ENGINE_PROTOCOL_ERROR', 'protocol'
    except OSError:
        code, stage = 'IO_ERROR', 'engine'
    except Exception:
        code, stage = 'ENGINE_PROTOCOL_ERROR', 'internal'
    emit(dict(protocol_version=PROTOCOL_VERSION, id=request_id, type='error',
              error=dict(code=code, message=code, stage=stage)))
    return 1


if __name__ == '__main__':
    os._exit(main())
