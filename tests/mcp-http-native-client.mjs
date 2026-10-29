// Connect only to the captured address of an explicitly isolated native acceptance app.
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {randomUUID} from 'node:crypto';
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {StreamableHTTPClientTransport} from '@modelcontextprotocol/sdk/client/streamableHttp.js';
const [captureFile, mode='readonly']=process.argv.slice(2);
assert(captureFile && ['readonly','write','unavailable'].includes(mode),'capture.json [readonly|write|unavailable]');
const capture=JSON.parse((await readFile(captureFile,'utf8')).replace(/^\uFEFF/,''));
assert.equal(capture.status,'PASSED'); const url=new URL(capture.url);
assert.equal(url.hostname,'127.0.0.1'); assert.equal(url.pathname,'/mcp');
const client=new Client({name:'yingqu-native-http-acceptance',version:'1.0'});
try{
  const connected=client.connect(new StreamableHTTPClientTransport(url),{timeout:3000});
  if(mode==='unavailable') await assert.rejects(connected);
  else{
    await connected;
    assert((await client.callTool({name:'list_categories',arguments:{}})).structuredContent.categories);
    const write=client.callTool({name:'create_category',arguments:{name:'HTTP 原生验收 '+randomUUID().slice(0,8)}});
    if(mode==='readonly')await assert.rejects(write,error=>error.message.includes('WRITE_DISABLED'));
    else assert((await write).structuredContent.id);
  }
  console.log('PASSED: native HTTP '+mode);
}finally{await client.close()}
