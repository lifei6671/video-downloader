播放器浏览器回归使用自生成素材，不包含平台视频，不进入应用安装包。

- `player.webm`：4 秒、160 × 90 的 VP8 测试图和静音 Vorbis 音轨，验证真实解码、拖动与资源释放。
- `audio-only.webm`：1 秒静音 Vorbis 音轨，验证只识别音频时切换到系统播放器。
- `player-portrait.mp4`：4 秒、90 × 160 的 H.264 测试图（无音轨），验证知识库纵向视频和不同编码的真实浏览器解码。由 FFmpeg 自生成，不含用户视频。

重新生成需要本机 FFmpeg，应用运行无需为此安装编码器：

```powershell
ffmpeg -f lavfi -i testsrc2=size=160x90:rate=10 -f lavfi -i anullsrc=r=48000:cl=mono -t 4 -c:v libvpx -b:v 80k -c:a libvorbis -q:a 2 -y tests/fixtures/media/player.webm
ffmpeg -f lavfi -i anullsrc=r=48000:cl=mono -t 1 -c:a libvorbis -q:a 2 -y tests/fixtures/media/audio-only.webm
ffmpeg -f lavfi -i testsrc2=size=90x160:rate=10 -t 4 -an -c:v libx264 -pix_fmt yuv420p -movflags +faststart -y tests/fixtures/media/player-portrait.mp4
```
