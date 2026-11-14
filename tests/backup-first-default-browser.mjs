// Production components with isolated IPC. No remote writes, real accounts or credentials.
async page => {
  const results=[];
  for(const firstProvider of ['telegram','s3'])for(const enabled of [false,true]){
    const p=await page.context().newPage();p.setDefaultTimeout(8000);
    const errors=[];p.on('pageerror',e=>errors.push(e.message));
    await p.addInitScript(enabled=>{
      window.spaces=[];window.current=null;window.syncEnabled=enabled;window.calls=[];
      window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener:()=>{}};
      window.__TAURI_INTERNALS__={convertFileSrc:v=>v,transformCallback:()=>1,unregisterCallback:()=>{},invoke:async(command,args={})=>{
        window.calls.push({command,activate:args.config?.activate});
        if(command==='get_settings')return {download_directory:'C:/fixture',filename_template:'{title}',max_downloads:2,profiles:[]};
        if(command==='get_backup_settings')return structuredClone(window.spaces.find(s=>s.space_id===window.current)??null);
        if(command==='get_backup_sync_enabled')return window.syncEnabled;
        if(command==='list_backup_spaces')return structuredClone(window.spaces);
        if(['list_tasks','list_video_transcripts','list_backup_assets','list_backup_operations'].includes(command))return [];
        if(command==='check_runtime_dependencies')return {platform:'windows',directory:'C:/fixture',tools:[]};
        if(command==='get_backup_credentials')return {access_key_id:'fixture-key',secret_access_key:'fixture-secret'};
        if(command==='telegram_storage_status')return {configured:true,phase:'ready',operation_id:'fixture-login',account:{id:'123',name:'测试账号'},last_error:null};
        if(command==='telegram_storage_channels')return [{id:'777',title:'测试私有频道',can_post:true,can_delete:false}];
        if(command==='save_backup_settings'){
          const id='space-'+(window.spaces.length+1);
          const {credentials,activate,create_library,...config}=args.config;
          if(window.spaces.length===0)window.current=id;
          const value={...config,space_id:id,library_id:'library-'+id,enabled:window.current===id&&window.syncEnabled,credentials_configured:config.provider_type==='s3',connection_pending:false,last_error:null,telegram_channel_title:config.provider_type==='telegram'?'测试私有频道':null};
          window.spaces.push(value);return structuredClone(value);
        }
        return null;
      }};
    },enabled);
    const button=name=>p.getByRole('button',{name,exact:true});
    const assert=(ok,message)=>{if(!ok)throw new Error(firstProvider+'/'+enabled+': '+message);};
    async function create(provider,name){
      await p.getByLabel('空间名称',{exact:true}).fill(name);
      await p.getByRole('combobox',{name:'备份提供商'}).click();await p.getByRole('option',{name:provider==='telegram'?'Telegram':'通用 S3',exact:true}).click();
      if(provider==='telegram'){
        await p.getByRole('combobox',{name:'私有频道'}).click();await p.getByRole('option',{name:'测试私有频道',exact:true}).click();
        await button('切换其他账号').waitFor();assert(await button('登录其他账号').count()===0,'Old account action remains');
        await button('保存配置').click();
      }else{
        await p.getByLabel('S3 Endpoint',{exact:true}).fill('https://fixture.s3.example.test');await p.getByLabel('Bucket',{exact:true}).fill(name==='首个备份'?'first':'second');
        await p.getByLabel('Prefix',{exact:true}).fill('media/');await p.getByLabel('Access Key ID',{exact:true}).fill('fixture-key');await p.getByLabel('Secret Access Key',{exact:true}).fill('fixture-secret');
        await button('新建配置').click();
      }
      await p.getByRole('tab',{name,exact:true}).waitFor();await button('删除配置').waitFor();
    }
    try{
      await p.goto(page.url().split('/').slice(0,3).join('/')+'/');await button('系统设置').click();await button('远端备份').click();
      await create(firstProvider,'首个备份');
      assert(await p.evaluate(()=>window.current)==='space-1','First save did not select the default');
      assert(await p.getByRole('tab',{name:'首个备份',exact:true}).getByText('默认',{exact:true}).count()===1,'Default badge missing');
      assert(await button('设为默认').count()===0,'First saved default still asks to be selected');
      assert(await p.evaluate(()=>window.syncEnabled)===enabled,'First default changed the sync preference');
      for(const [width,height]of [[1440,900],[1280,800],[1024,680]]){
        await p.setViewportSize({width,height});assert(await p.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'Horizontal overflow');
      }
      await p.getByRole('tab',{name:'新建空间',exact:true}).click();
      await create(firstProvider==='telegram'?'s3':'telegram','后续备份');
      assert(await p.evaluate(()=>window.current)==='space-1','Later save replaced the existing default');
      assert(await button('设为默认').isVisible(),'Later configuration lost manual selection');
      const calls=await p.evaluate(()=>window.calls);
      assert(calls.filter(c=>c.command==='save_backup_settings').every(c=>c.activate===false),'UI forced activation');
      assert(!calls.some(c=>c.command==='select_backup_space'),'UI selected the default separately from saving');
      assert(errors.length===0,errors.join('\n'));results.push({firstProvider,enabled,viewports:3,passed:true});
    }finally{await p.close();}
  }
  return {passed:true,results};
}
