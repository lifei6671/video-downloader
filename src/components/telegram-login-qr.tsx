export function TelegramLoginQr({ qr }: { qr: boolean[][] }) {
  return <div className="mx-auto w-full max-w-64 rounded-lg bg-card p-2"><svg role="img" aria-label="Telegram 登录二维码" className="block aspect-square w-full" viewBox={`0 0 ${qr.length} ${qr.length}`} shapeRendering="crispEdges"><rect width={qr.length} height={qr.length} className="fill-card" /><path className="fill-foreground" d={qr.flatMap((row, y) => row.flatMap((black, x) => black ? [`M${x} ${y}h1v1h-1z`] : [])).join('')} /></svg></div>;
}
