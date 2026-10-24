// Requires the isolated me.disign.yingqu.cuda-validation-20261023 fixture app; writes its known test text to the clipboard.
async page => {
 page.setDefaultTimeout(15000);
 await page.getByRole('button',{name:'下载任务',exact:true}).click();
 await page.getByText('Fun-ASR-Nano 本机真实验证',{exact:true}).waitFor();
 await page.locator('.task-row').filter({has:page.getByText('Fun-ASR-Nano 本机真实验证',{exact:true})}).click({button:'right',position:{x:200,y:30}});
    await page.getByRole('menuitem',{name:'提取文案',exact:true}).click();
 const dialog=page.getByRole('dialog',{name:'视频文案'});
 await dialog.getByRole('button',{name:/^(开始提取|重新提取)$/}).click();
 const text=dialog.getByLabel('识别文案',{exact:true});await text.waitFor({timeout:60000});
 if(!/开放时间早上(?:9|九)点/.test(await text.innerText()))throw Error('unexpected native ASR output');
 await dialog.getByRole('button',{name:'复制全文',exact:true}).click();
 await page.getByText('文案已复制',{exact:true}).waitFor();
 await page.screenshot({path:'.local/transcription-delivery/native-dialog.png'});
 const snapshot=await page.evaluate(()=>window.__TAURI_INTERNALS__.invoke('get_video_transcript',{taskId:'asr-native-test'}));
 const tasks=await page.evaluate(()=>window.__TAURI_INTERNALS__.invoke('list_tasks'));
 if(snapshot.state!=='completed' || tasks[0].status!=='completed')throw Error('native persistence/state failed');
 await page.keyboard.press('Escape');
 await page.locator('.task-row').filter({has:page.getByText('Fun-ASR-Nano 本机真实验证',{exact:true})}).click({button:'right',position:{x:200,y:30}});
    await page.getByRole('menuitem',{name:'查看文案',exact:true}).click();await text.waitFor();
 return {nativeTauri:'PASS',copy:'PASS',text:await text.innerText(),state:snapshot.state,model:snapshot.result.model_revision,reopen:'PASS'};
}
