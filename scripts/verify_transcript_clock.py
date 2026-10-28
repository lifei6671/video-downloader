"""WB-302 production extraction clock matrix using installed FFmpeg/FFprobe."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import wave

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'python-engine'))
import local_tools
import transcription


def pcm(path):
    with wave.open(str(path), 'rb') as source:
        assert (source.getframerate(), source.getnchannels(), source.getsampwidth()) == (16000, 1, 2)
        return source.readframes(source.getnframes())


def first_signal(data):
    return next(i // 2 for i in range(0, len(data), 2)
                if abs(int.from_bytes(data[i:i + 2], 'little', signed=True)) > 100)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    output = args.output.resolve()
    if output == ROOT / '.local' or not output.is_relative_to(ROOT / '.local'):
        parser.error('Output must be a new directory below repository .local')
    ffmpeg = local_tools.installed_tool('ffmpeg')
    if not ffmpeg:
        parser.error('Existing FFmpeg required; no installation')
    ffprobe = Path(ffmpeg).with_name('ffprobe.exe' if sys.platform == 'win32' else 'ffprobe')
    if not ffprobe.is_file():
        parser.error('Existing sibling FFprobe required for this test, not a product dependency')
    output.mkdir(parents=True, exist_ok=False)
    audio = ROOT / 'python-engine/assets/sensevoice-smoke.wav'
    source = pcm(audio)
    rows = []
    with (output / 'ffmpeg.log').open('wb') as log:
        def run(arguments):
            subprocess.run([ffmpeg, '-nostdin', '-v', 'error', '-n', *arguments], stdin=subprocess.DEVNULL,
                           stdout=subprocess.DEVNULL, stderr=log, check=True, timeout=30,
                           creationflags=0x08000000 if sys.platform == 'win32' else 0)
        cases = [('zero', 0, 0, 'mkv', 'pcm_s16le'), ('delay', 1.5, 0, 'mkv', 'pcm_s16le'),
                 ('origin', 1.5, 5, 'mkv', 'pcm_s16le'), ('early', -.5, 0, 'mkv', 'pcm_s16le'),
                 ('mov-delay', 1.5, 0, 'mov', 'pcm_s16le'), ('mp4-zero', 0, 0, 'mp4', 'aac'),
                 ('mp4-delay', 1.5, 0, 'mp4', 'aac'), ('mp4-origin', 1.5, 5, 'mp4', 'aac'),
                 ('vfr-delay', 1.5, 0, 'mkv', 'pcm_s16le'), ('webm-zero', 0, 0, 'webm', 'libopus'),
                 ('webm-delay', 1.5, 0, 'webm', 'libopus')]
        for name, offset, origin, extension, codec in cases:
            video = output / f'{name}.{extension}'
            video_codec = 'libvpx-vp9' if extension == 'webm' else 'libx264'
            video_options = ['-vf', 'select=not(mod(n\\,3))+not(mod(n\\,7))', '-fps_mode', 'vfr'] if name == 'vfr-delay' else []
            run(['-f', 'lavfi', '-i', 'color=c=black:s=160x90:r=10:d=6', '-itsoffset', str(offset), '-i', str(audio),
                 '-map', '0:v:0', '-map', '1:a:0', '-c:v', video_codec, *([] if extension == 'webm' else ['-bf', '0']),
                 *video_options, '-c:a', codec, '-avoid_negative_ts', 'disabled', '-output_ts_offset', str(origin), str(video)])
            info = json.loads(subprocess.check_output([str(ffprobe), '-v', 'error', '-show_entries',
                'stream=codec_type,start_time:format=start_time,duration', '-of', 'json', str(video)], timeout=30))
            audio_start = float(next(s for s in info['streams'] if s['codec_type'] == 'audio')['start_time'])
            legacy, normalized = output / f'legacy-{name}.wav', output / f'normalized-{name}.wav'
            # Match production input restrictions, selected stream, rate and output format.
            base = ['-protocol_whitelist', 'file', '-format_whitelist', 'mov,matroska,webm', '-i', str(video),
                    '-map', '0:a:0', '-vn', '-ac', '1', '-ar', '16000', '-c:a', 'pcm_s16le']
            run([*base, '-f', 'wav', str(legacy)])
            production = transcription.audio_command(ffmpeg, video, normalized)
            subprocess.run(production, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=log, check=True, timeout=30,
                           creationflags=0x08000000 if sys.platform == 'win32' else 0)
            padding = round(max(offset + origin, 0) * 16000)
            trim = round(max(-(offset + origin), 0) * 16000)
            data = pcm(normalized)
            if codec == 'pcm_s16le':
                assert pcm(legacy) == source
                assert data == bytes(padding * 2) + source[trim * 2:], 'Production extraction lost media origin'
            reference = source[trim * 2:]
            error_frames = abs(first_signal(data) - first_signal(reference) - padding)
            tolerance_frames = 1024 if codec == 'aac' else 320 if codec == 'libopus' else 0
            assert error_frames <= tolerance_frames, 'Codec onset differs from presentation timeline'
            row = dict(name=name, declared_audio_offset=offset, container_origin=origin, audio_start_seconds=audio_start,
                       video_probe=info, normalized_frames=len(data) // 2, expected_padding_ms=padding // 16,
                       pcm_exact=codec == 'pcm_s16le', onset_error_frames=error_frames, codec_tolerance_frames=tolerance_frames)
            if name == 'vfr-delay':
                stamps = json.loads(subprocess.check_output([str(ffprobe), '-v', 'error', '-select_streams', 'v:0',
                    '-show_entries', 'frame=pts_time', '-of', 'json', str(video)], timeout=30))
                points = [round(float(f['pts_time']) * 1000) for f in stamps['frames']]
                assert len({b - a for a, b in zip(points, points[1:])}) > 1, 'Fixture must be variable frame rate'
                row['video_pts_ms'] = points
            rows.append(row)
    (output / 'summary.json').write_text(json.dumps(dict(status='PASSED', cases=rows,
        limitation='Controlled offsets/containers/codecs only; signal onset tolerance is not word alignment accuracy',
        productionCommand=True), ensure_ascii=False, indent=2), encoding='utf-8')
    print(f'PASSED: {len(rows)} production media-clock cases')


if __name__ == '__main__':
    main()
