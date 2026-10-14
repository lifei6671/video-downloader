"""Isolated test entry point: synthetic metadata, real yt-dlp HTTP downloader."""
import os
import sys
from pathlib import Path
from urllib.parse import urlsplit

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'python-engine'))
import engine
from yt_dlp import YoutubeDL


class FixtureYDL(YoutubeDL):
    def extract_info(self, url, download=True, **kwargs):
        platform = 'douyin' if 'douyin.com' in url else 'tiktok'
        mode = os.environ.get('FIXTURE_MODE', 'good')
        video_id = urlsplit(url).path.rstrip('/').split('/')[-1]
        if mode == 'wrong_identity':
            video_id = '999'
        domain = 'douyinvod.com' if platform == 'douyin' else 'tiktokcdn.com'
        info = {'id':video_id, 'title':'fixture', 'uploader':'fixture',
                'webpage_url':url, 'extractor':'fixture', 'extractor_key':'Fixture',
                'formats':[{'format_id':'single', 'url':f'http://v.fixture.{domain}/{mode}.mp4',
                            'ext':'mp4','protocol':'http','vcodec':'h264','acodec':'aac'}]}
        if mode == 'playlist':
            return {'_type':'playlist','entries':[]}
        if mode == 'segmented':
            info['formats'][0]['protocol'] = 'm3u8_native'
        return self.process_ie_result(info, download=download)


if __name__ == '__main__':
    if os.environ.get('FIXTURE_PID_FILE'):
        Path(os.environ['FIXTURE_PID_FILE']).write_text(str(os.getpid()))
    engine.YoutubeDL = FixtureYDL
    engine.load_default_cookies = lambda *_: None
    from bridge import main
    os._exit(main())
