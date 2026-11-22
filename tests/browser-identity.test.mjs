import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';

const script=readFileSync(new URL('../src-tauri/src/browser-identity.js',import.meta.url),'utf8');
for(const mode of ['chromium','webkit','hints-denied']) {
  test(`native browser identity: ${mode}`,async()=>{
    const navigator={userAgent:'Native browser UA',platform:'Win32',language:'zh-CN',hardwareConcurrency:24};
    if(mode!=='webkit') navigator.userAgentData={brands:[{brand:'Real browser',version:'149'}],mobile:false,platform:'Windows',
      getHighEntropyValues:async()=>{
        if(mode==='hints-denied')throw Error('denied');
        return {platformVersion:'19.0.0',architecture:'x86',bitness:'64',fullVersionList:[{brand:'Real browser',version:'149.0.1'}]};
      }};
    const before=navigator.userAgent;
    const context={window:{},navigator,screen:{width:2560,height:1440},Intl};
    vm.runInNewContext(script,context);
    await new Promise(resolve=>setImmediate(resolve));
    const value=JSON.parse(JSON.stringify(context.window.__yingquBrowserIdentity));
    assert.equal(value.user_agent,before);
    assert.equal(navigator.userAgent,before);
    assert.equal(value.environment.screen_width,2560);
    assert.equal(value.environment.hardware_concurrency,24);
    assert.equal(value.environment.device_memory,null);
    if(mode==='webkit')assert.deepEqual(value.client_hints,{});
    else assert.equal(value.client_hints['sec-ch-ua-platform'],'"Windows"');
    assert.equal(value.client_hints['sec-ch-ua-platform-version'],mode==='chromium'?'"19.0.0"':undefined);
    assert(!Object.keys(value).some(key=>/cookie|token|password/i.test(key)));
  });
}
