const {createRequire}=require('node:module');const path=require('node:path');const fs=require('node:fs');
const local=createRequire(path.resolve('package.json'));const {Builder}=local('selenium-webdriver');const chrome=local('selenium-webdriver/chrome');
(async()=>{
 const options=new chrome.Options().setChromeBinaryPath(process.env.CHROME_BIN).addArguments('--headless','--no-sandbox');
 const driver=await new Builder().forBrowser('chrome').setChromeOptions(options).setChromeService(new chrome.ServiceBuilder(process.env.CHROMEDRIVER_BIN)).build();
 try{
  await driver.manage().setTimeouts({script:240000});await driver.get(process.env.MOPRO_GNARK_BENCH_URL);
  const result=await driver.executeAsyncScript(function(threads,done){(async()=>{
   const backend=await import('./MoproWasmBindings/gnark/gnark.backend.js');await backend.initAccelerator({experimental:true,threads});
   const Selected=globalThis.__moproGnarkKernel.Key;
   const ref=await import('/arkworks/web/MoproWasmBindings/gnark/accelerator/gnark_kernel.js');
   if(globalThis.__moproBenchRuntime.backend!=='arkworks'){await ref.default();await ref.initThreadPool(threads);}
   const names=await(await fetch('/vectors/index.json')).json();let checks=0;
   const bytes=hex=>Uint8Array.from(hex.match(/../g)||[],x=>parseInt(x,16));
   const eq=(a,b,label)=>{if(a.length!==b.length||a.some((x,i)=>x!==b[i]))throw new Error(label+' differs');checks++;};
   for(const name of names){
    const v=await(await fetch('/vectors/'+name)).json();const args=['a','b','k','z','b2','params'].map(k=>bytes(v[k]));
    const reference=new ref.Key(...args), selected=new Selected(...args), second=new Selected(...args);
    for(const key of [reference,selected,second])key.add_commitment(args[0],args[0]);
    await selected.initialize?.();await second.initialize?.();
    try{
     for(let round=0;round<3;round++){
      const scalars=bytes(v.scalars[round]), values=v.evaluations.map(bytes);
      const expected=reference.parts(scalars,scalars,scalars,...values);
      eq(await selected.parts(scalars,scalars,scalars,...values),expected,name+' parts '+round);
      for(const knowledge of [false,true])eq(await selected.commitment(0,knowledge,scalars),reference.commitment(0,knowledge,scalars),name+' commitment '+round);
      eq(await second.parts(scalars,scalars,scalars,...values),expected,name+' second handle '+round);
     }
     selected.free();
     // Disposal of a different key must preserve the remaining prepared key.
     const scalars=bytes(v.scalars[0]),values=v.evaluations.map(bytes);
     eq(await second.parts(scalars,scalars,scalars,...values),reference.parts(scalars,scalars,scalars,...values),name+' surviving handle');
     let failed=false;try{await second.commitment(0,false,new Uint8Array(v.n*32+1));}catch{failed=true;}
     if(!failed)throw new Error('accepted malformed scalar length');
    }finally{second.free();reference.free();}
   }
   return{...globalThis.__moproBenchRuntime,threads,cases:names.length,checks};
  })().then(done,e=>done({error:String(e),stack:e.stack}));},Number(process.env.MOPRO_GNARK_THREADS||16));
  fs.writeFileSync(process.env.MOPRO_GNARK_BENCH_REPORT,JSON.stringify(result,null,2));console.log(result);if(result.error)process.exitCode=1;
 }finally{await driver.quit();}
})().catch(e=>{console.error(e);process.exitCode=1});
