// Production UI and isolated IPC only. No real accounts, QR authorization or vault access.
async page => {
  const results=[];
  for (const purpose of ['download','backup']) for (const [width,height] of [[1440,900],[1280,800],[1024,680]]) {
    const p=await page.context().newPage(); p.setDefaultTimeout(8000); await p.setViewportSize({width,height});
    const errors=[]; p.on('pageerror',error=>errors.push(error.message));
    await p.addInitScript(() => {
      const ready=name=>({configured:true,phase:'ready',operation_id:'operation-'+name,account:{id:name,name},last_error:null,qr:null});
      const out=()=>({configured:true,phase:'signed_out',operation_id:'old-restore',account:null,last_error:null,qr:null});
      window.ready=ready;window.out=out;window.auth={download:ready('下载账号 A'),backup:ready('备份账号 A')};window.pending={};window.calls=[];window.channelReads=0;
      window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener:()=>{}};
      window.__TAURI_INTERNALS__={convertFileSrc:v=>v,transformCallback:()=>1,unregisterCallback:()=>{},invoke:async(command,args={})=>{
        const purpose=args.purpose??'backup',other=purpose==='download'?'backup':'download';
        window.calls.push({command,purpose,otherAccount:args.otherAccount});
        const view=()=>structuredClone(window.pending[purpose]??window.auth[purpose]);
        if(command==='get_settings')return {download_directory:'C:/fixture',filename_template:'{title}',max_downloads:2,profiles:[{platform:'telegram',user_agent:'fixture',proxy:null,cookie_configured:false}]};
        if(command==='get_backup_settings')return null;
        if(command==='get_backup_sync_enabled')return false;
        if(['list_tasks','list_video_transcripts','list_backup_spaces','list_backup_assets','list_backup_operations'].includes(command))return [];
        if(command==='check_runtime_dependencies')return {platform:'windows',directory:'C:/fixture',tools:[]};
        if(command==='telegram_storage_status')return view();
        if(command==='telegram_storage_resume'){window.auth[purpose]=out();return view();}
        if(command==='telegram_storage_channels'){window.channelReads++;return [{id:'777',title:'当前账号私有频道',can_post:true,can_delete:false}];}
        if(command==='telegram_storage_login_qr'){
          if(!args.otherAccount && window.auth[other].phase==='ready'){window.auth[purpose]=window.auth[other];return view();}
          window.pending[purpose]={...out(),phase:'qr',operation_id:'qr-'+purpose,qr:[[true,false],[false,true]]};
          return await new Promise((resolve,reject)=>{
            window.failQr=loseOriginal=>{delete window.pending[purpose];if(loseOriginal)window.auth[purpose]=out();reject('TELEGRAM_QR_TIMEOUT');};
            window.finishQr=name=>{window.auth[purpose]=ready(name);delete window.pending[purpose];resolve(view());};
            window.releaseQr=resolve;
          });
        }
        if(command==='telegram_storage_cancel'){
          delete window.pending[purpose];
          if(window.cancelCommitted){window.auth[purpose]=ready('已完成登录');window.releaseQr?.(view());throw 'TELEGRAM_STALE_OPERATION';}
          window.releaseQr?.(view());return view();
        }
        if(command==='telegram_storage_logout'){
          if(window.failLogout)throw 'TELEGRAM_CONNECTION_ERROR';window.auth[purpose]=out();return view();
        }
        return null;
      }};
    });
    const button=name=>p.getByRole('button',{name,exact:true});
    const text=name=>p.getByText(name,{exact:true}).filter({visible:true});
    const assert=(value,message)=>{if(!value)throw new Error(purpose+': '+message);};
    const label=purpose==='download'?'Telegram 账号':'Telegram 存储账号';
    const logout=purpose==='download'?'退出账号':'退出存储账号';
    const account=purpose==='download'?'下载账号 A':'备份账号 A';
    const idle=()=>p.locator('section[aria-label="'+label+'"][aria-busy="false"]').waitFor();
    try {
      await p.goto(page.url().split('/').slice(0,3).join('/')+'/');await button('系统设置').click();
      if(purpose==='download'){await button('平台账号').click();await p.getByRole('tab',{name:'Telegram',exact:true}).click();}
      else {await button('远端备份').click();await p.getByRole('combobox',{name:'备份提供商'}).click();await p.getByRole('option',{name:'Telegram',exact:true}).click();}
      await text(account).waitFor();await idle();
      const accountCard=p.locator('section[aria-label="Telegram 账号"] [data-slot="card"]');
      if(purpose==='download') {
        assert(await accountCard.count()===1,'Telegram account should have one settings card');
        assert(await accountCard.getByText(account,{exact:true}).isVisible(),'Logged-in account is outside the settings card');
        assert(await accountCard.getByRole('button',{name:logout,exact:true}).isVisible(),'Logout is outside the settings card');
      }
      const reads=await p.evaluate(()=>window.channelReads);
      await button('切换其他账号').click();await p.getByRole('img',{name:'Telegram 登录二维码'}).waitFor();
      await p.evaluate(()=>window.failQr(false));await text(account).waitFor();
      if(purpose==='backup')await p.waitForFunction(before=>window.channelReads>before,reads);
      await idle();assert(await p.getByRole('alert').filter({hasText:'扫码登录等待超过'}).isVisible(),'Automatic channel refresh erased failed-switch feedback');

      await button('切换其他账号').click();await p.getByRole('img',{name:'Telegram 登录二维码'}).waitFor();
      await p.evaluate(()=>window.failQr(true));await button('重新扫码').waitFor();await button('重新扫码').click();
      await p.getByRole('img',{name:'Telegram 登录二维码'}).waitFor();
      assert(await p.evaluate(()=>window.calls.filter(c=>c.command==='telegram_storage_login_qr').at(-1).otherAccount===true),'QR retry reused another account instead of keeping the requested switch');
      await p.evaluate(()=>window.finishQr('新登录账号'));await text('新登录账号').waitFor();await idle();

      // Background SDK authorization loss must update an already-visible account page.
      await p.evaluate(purpose=>{window.auth[purpose]={...window.out(),last_error:'TELEGRAM_AUTH_REQUIRED'};},purpose);
      await button('扫码登录').waitFor();assert(await button(logout).count()===0,'Expired authorization still exposes logout');
      assert(await text('新登录账号').count()===0,'Expired account remains displayed');
      if(purpose==='backup')assert(await p.getByRole('combobox',{name:'私有频道'}).isDisabled(),'Expired authorization retained selectable channels');
      await button('恢复登录').click();await text('没有可恢复的登录，请扫码登录。').waitFor();
      await p.evaluate(purpose=>{window.auth[purpose]=window.ready('后台恢复账号');},purpose);
      await text('后台恢复账号').waitFor();
      assert(await text('没有可恢复的登录，请扫码登录。').count()===0,'Recovered account retained an obsolete restore notice');await idle();

      // A failed cancel must reconcile with a login that has already committed in Rust.
      await button('切换其他账号').click();await p.getByRole('img',{name:'Telegram 登录二维码'}).waitFor();
      await p.evaluate(()=>{window.cancelCommitted=true;});await p.keyboard.press('Escape');
      await text('已完成登录').waitFor();await idle();
      assert(await button('继续登录').count()===0,'Failed cancel left a stale QR state');
      await p.evaluate(()=>{window.failLogout=true;});await button(logout).click();
      await p.getByRole('alert').filter({hasText:'连接失败'}).waitFor();await text('已完成登录').waitFor();
      assert(await button(logout).isEnabled(),'Failed logout incorrectly removed the active account');
      await p.evaluate(()=>{window.failLogout=false;});await button(logout).click();await button('扫码登录').waitFor();
      assert(await button(logout).count()===0,'Signed-out account exposes logout');
      if(purpose==='download') {
        assert(await accountCard.getByRole('button',{name:'扫码登录',exact:true}).isVisible(),'Signed-out controls are outside the settings card');
        assert(await accountCard.getByRole('button',{name:'恢复登录',exact:true}).isVisible(),'Restore login is outside the settings card');
      }
      assert(await p.locator('form form').count()===0,'Nested account/proxy forms');
      assert(await p.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'Horizontal overflow');
      assert(errors.length===0,errors.join('\n'));
      results.push({purpose,width,height,passed:true,flows:['failed-switch-feedback','retry-other-account','background-expiry','background-restore','cancel-after-commit','logout-retry']});
    } finally {await p.close();}
  }
  return {passed:true,results};
}
