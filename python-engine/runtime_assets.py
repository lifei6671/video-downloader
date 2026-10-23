"""Pinned runtime downloads; provenance and licenses: vendor/UPSTREAM.md."""

FFMPEG_BASE = 'https://github.com/eugeneware/ffmpeg-static/releases/download/b6.1.1/'
DENO_BASE = 'https://github.com/denoland/deno/releases/download/v2.9.7/'
# Each tuple is (download size, SHA-256); hashes cover the downloaded archive.
FFMPEG = {
    'win32-x64': [
        (29581307, '8883a3dffbd0a16cf4ef95206ea05283f78908dbfb118f73c83f4951dcc06d77'),
        (29521644, 'f309e6223ad89d2fe54bccd420a7709b66fd27540674e92309578ed491a43c8d'),
        (35147, '8ceb4b9ee5adedde47b31e975c1d90c73ad27b6b165a1dcd80c7c545eb65b903'),
        (39494, 'a636a7183c58006351acbaf35303c0ed85c6e1320fd4e80de453ba6157de6311')],
    'darwin-x64': [
        (25296431, '929b375c1182d956c51f7ac25e0b2b0411fb01f6f407aa15c9758efeb4242106'),
        (25239438, 'd4da574d6e2e197bd259b47d69cf262df9e312af24ad960444f6d806d3d4c186'),
        (4346, '2e1d16c72fd74e12063776371da757322f8b77589386532f4fd8634bde7de1af'),
        (6227, 'e88a0325f8e5b75210355e37341824f074d3cd82def2125be54c914b62848a36')],
    'darwin-arm64': [
        (19246198, '8923876afa8db5585022d7860ec7e589af192f441c56793971276d450ed3bbfa'),
        (19207077, 'd986a8ec7b030899fe66a8a288ed809a3543338705a3ce178cfb85869c5d80be'),
        (4376, 'cb48bf09a11f5fb576cddb0431c8f5ed0a60157a9ec942adffc13907cbe083f2'),
        (1810, '05ba4b92c96605434b1aaae3eedf5a2c280c9607bf78ffca9a5b536d9af2dc6a')],
}
DENO = {
    'win32-x64': ('x86_64-pc-windows-msvc', 42630221, 'a0c3101b4158d1dfb7d6a78a7bf0f3de80c96bb423c152beec8beb22786f2238'),
    'darwin-x64': ('x86_64-apple-darwin', 42295422, '95daaff11c116a52ad54785e7914c8e9c9cdcaba793c5ed929c74ca2d8e6259a'),
    'darwin-arm64': ('aarch64-apple-darwin', 38469316, '5cd46d6268f6f78f5d88bdc7159d20bd44cdaa4b3303474839f87ec6fe7ae25c'),
}


def assets(tool, platform):
    """Return fixed URL, output name, compression, size, digest tuples."""
    suffix = '.exe' if platform.startswith('win32-') else ''
    if tool == 'fun-asr-nano' and platform in FFMPEG:
        from transcription import MODEL_REVISION, VAD_REVISION
        return [
            (f'https://huggingface.co/FunAudioLLM/Fun-ASR-Nano-GGUF/resolve/{MODEL_REVISION}/funasr-encoder-f16.gguf',
             'funasr-encoder-f16.gguf', 'raw', 469331008, 'f92f91d01a24fbed6c863495b2ee8c6a6788144a02858b75743f0946668de8a2'),
            (f'https://huggingface.co/FunAudioLLM/Fun-ASR-Nano-GGUF/resolve/{MODEL_REVISION}/qwen3-0.6b-q8_0.gguf',
             'qwen3-0.6b-q8_0.gguf', 'raw', 804753280, '819f385dc0e035dccc3d9e7edaf6b7b044b8ba7ace63cbcbf84c7e397eecbf27'),
            (f'https://huggingface.co/FunAudioLLM/fsmn-vad-GGUF/resolve/{VAD_REVISION}/fsmn-vad.gguf',
             'fsmn-vad.gguf', 'raw', 1720512, '1270f2559c495f4e7b6e739541151027d360761a3fda43fc147034f5719f5479')]
    if tool == 'ffmpeg' and platform in FFMPEG:
        names = [(f'ffmpeg-{platform}.gz', 'ffmpeg' + suffix, 'gzip'),
                 (f'ffprobe-{platform}.gz', 'ffprobe' + suffix, 'gzip'),
                 (f'{platform}.LICENSE', 'LICENSE', 'raw'),
                 (f'{platform}.README', 'UPSTREAM-README', 'raw')]
        return [(FFMPEG_BASE + name, output, kind, *digest)
                for (name, output, kind), digest in zip(names, FFMPEG[platform])]
    if tool == 'javascript' and platform in DENO:
        target, size, digest = DENO[platform]
        return [(DENO_BASE + f'deno-{target}.zip', 'deno' + suffix, 'zip', size, digest),
                ('https://raw.githubusercontent.com/denoland/deno/v2.9.7/LICENSE.md',
                 'LICENSE', 'raw', 1074, 'f62497fffecc0852960c8d3e6934b9db86d16396e9b604072e923892cae3a588')]
    return []
