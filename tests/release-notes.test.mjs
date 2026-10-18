import assert from 'node:assert/strict';
import { after, test } from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

const server = await createServer({ server: { middlewareMode: true }, appType: 'custom' });
after(() => server.close());
const { ReleaseNotes } = await server.ssrLoadModule('/src/components/release-notes.tsx');
const render = content => renderToStaticMarkup(createElement(ReleaseNotes, { content, openLink: () => {} }));

test('release notes render Markdown and GitHub tables/task lists', () => {
  const html = render('### 新增功能\n\n- **检查更新**：支持 `Markdown`。\n\n> 提示\n\n1. 第一步\n2. 第二步\n\n| 平台 | 支持 |\n| --- | --- |\n| Windows | 是 |\n\n- [x] 已完成\n- [ ] 待完成\n\n~~旧说明~~');
  for (const expected of ['<h3>新增功能</h3>', '<strong>检查更新</strong>', '<code>Markdown</code>', '<ul>', '<ol>', '<blockquote>', '<table>', '<th>平台</th>', '<td>Windows</td>', '<del>旧说明</del>']) assert.ok(html.includes(expected), expected);
  assert.match(html, /type="checkbox" disabled="" checked=""/);
  assert.match(html, /type="checkbox" disabled=""/);
});

test('release notes skip raw HTML, escape code and disable non-web links', () => {
  const html = render('<script>alert(1)</script>\n\n<img src="https://example.com/track" onerror="alert(1)">\n\n[危险](javascript:alert%281%29) [文件](file:///C:/private) [相对](./secret) [安全](https://example.com/notes) [HTTP](http://example.com/notes)\n\n![示意图](https://example.com/image.png)\n\n```html\n<img onerror="alert(1)">\n```');
  assert.doesNotMatch(html, /<script|<img|href="(?:javascript:|file:|\.\/)/);
  assert.match(html, /<span>危险<\/span>/);
  assert.match(html, /<span>示意图<\/span>/);
  assert.match(html, /href="https:\/\/example.com\/notes" target="_blank" rel="noreferrer"/);
  assert.match(html, /href="http:\/\/example.com\/notes"/);
  assert.match(html, /&lt;img onerror=/);
});

test('empty release notes retain the existing placeholder and keyboard scroll target', () => {
  const html = render(' \n\t ');
  assert.match(html, /tabindex="0" aria-label="更新说明内容"/);
  assert.match(html, /此版本未提供更新说明。/);
});
