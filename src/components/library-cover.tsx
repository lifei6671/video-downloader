import { useEffect, useState } from 'react';
import { Film } from 'lucide-react';
import { cn } from 'cn';
import { libraryApi, type LibraryAsset } from '@/lib/library-api';

export function LibraryCover({ asset, className }: { asset: LibraryAsset; className?: string }) {
  const [source, setSource] = useState<string>();
  useEffect(() => {
    let disposed = false; let url: string | undefined;
    setSource(undefined);
    void libraryApi.cover(asset.id).then(data => {
      if (disposed) return;
      const bytes = new Uint8Array(data);
      url = URL.createObjectURL(new Blob([bytes], { type: bytes[0] === 0xff ? 'image/jpeg' : bytes[0] === 0x89 ? 'image/png' : 'image/webp' }));
      setSource(url);
    }).catch(() => { /* Missing cover is represented by the media placeholder. */ });
    return () => { disposed = true; if (url) URL.revokeObjectURL(url); };
  }, [asset.id, asset.updated_at]);
  return <div className={cn('library-cover', className)}>{source ? <img src={source} alt="" onError={() => setSource(undefined)} /> : <Film aria-label="暂无封面" />}</div>;
}
