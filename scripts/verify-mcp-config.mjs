// Validate a copied HTTP address with the official SDK; no command helper is launched.
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {StreamableHTTPClientTransport} from '@modelcontextprotocol/sdk/client/streamableHttp.js';
import {allTools} from './mcp-client-checks.mjs';
const args=process.argv.slice(2);
assert(args.length>=1 && args.length<=2 && !args[0].startsWith('--') && (args.length===1 || args[1]==='--unavailable'),
  'Usage: node scripts/verify-mcp-config.mjs capture.json [--unavailable]');
const capture=JSON.parse((await readFile(args[0],'utf8')).replace(/^\uFEFF/,''));
assert.equal(capture.status,'PASSED');
const endpoint=capture.url??capture.info.status.endpoint;
assert.equal(endpoint,capture.info.status.endpoint);
const address=new URL(endpoint); assert.equal(address.hostname,'127.0.0.1'); assert.equal(address.pathname,'/mcp');
if(args.includes('--unavailable')){
  await assert.rejects(fetch(endpoint,{signal:AbortSignal.timeout(2000)}));
}else{
  const client=new Client({name:'yingqu-copied-http-url',version:'1.0'});
  try{
    await client.connect(new StreamableHTTPClientTransport(address),{timeout:10000});
    assert.deepEqual((await client.listTools()).tools.map(tool=>tool.name),allTools);
    assert((await client.callTool({name:'list_categories',arguments:{}})).structuredContent.categories);
  }finally{await client.close()}
}
console.log('PASSED: copied Streamable HTTP URL');
