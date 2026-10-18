import Markdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

export function ReleaseNotes({ content, openLink }: { content: string; openLink: (url: string) => void }) {
  return <div tabIndex={0} aria-label="更新说明内容" className="release-notes max-h-[25dvh] shrink-0 overflow-auto break-words rounded-md border bg-muted/50 p-3 text-sm leading-relaxed outline-none focus-visible:ring-2 focus-visible:ring-ring">
    <Markdown skipHtml remarkPlugins={[remarkGfm]} urlTransform={url => /^https?:\/\//i.test(url) ? url : undefined} components={{
      a: ({ href, children }) => href ? <a href={href} target="_blank" rel="noreferrer" onClick={event => { event.preventDefault(); openLink(href); }}>{children}</a> : <span>{children}</span>,
      img: ({ alt }) => <span>{alt}</span>,
    }}>{content.trim() || '此版本未提供更新说明。'}</Markdown>
  </div>;
}
