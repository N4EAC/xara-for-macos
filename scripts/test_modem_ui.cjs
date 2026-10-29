// Run with NODE_PATH pointing to a Playwright installation and the local UI server running.
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
(async () => {
  const browser = await chromium.launch({headless:true});
  try {
    const page = await browser.newPage({viewport:{width:1440,height:1180}, deviceScaleFactor:1});
    const errors=[]; page.on('pageerror',e=>errors.push(e.message));
    await page.addInitScript(() => {
      window.commands=[];
      window.WebSocket=class {
        static OPEN=1;
        constructor() { this.readyState=1; window.socket=this; setTimeout(()=>this.onopen?.(),0); }
        send(cmd) { window.commands.push(cmd); }
      };
      window.deliver=m=>window.socket.onmessage({data:JSON.stringify(m)});
    });
    await page.goto(process.env.XARA_UI_URL || 'http://127.0.0.1:8765');
    await page.evaluate(() => {
      deliver({t:'hello',mode:'hf',control:true,locked:false,drive_min:-30,meter_floor:-120,span:3000,version:'test-1.2',v:1,replay:12,tune_limit:20});
      deliver({t:'settings',available:true,path:'/modem/ui.conf',saved:{call:'W1AW'},values:{hf:'2300',capture:'radio-in',playback:'radio-out'}});
      deliver({t:'devices',backend:'coreaudio',capture:[],playback:[]});
      window.sample={t:'status',ts:100,calls:['W1AW'],state:'Receiving',peer:'K6XAR',bw:'2300',level:7,bitrate:{bps:2412,tx:false,level:9},sn:14.2,afc:3.5,attempts:1,retries:15,buffer:128,received_bytes:24832,ptt:false,busy:true,rx:{rms:-18,peak:-1.5,clip:3},tx:{rms:-30,peak:-6,clip:2},passband:[350,2650],drive:-5,tuning:false,dropped:7};
      deliver(sample);
      window.statusTimer=setInterval(()=>{sample.ts+=.1;deliver(sample)},100);
      deliver({t:'waterfall',row:1,span:3000,bins:512,step:.5,rate:10,floor:-128});
      for(let y=0;y<200;y++) {
        const row=new Uint8Array(516);row[0]=1;row[2]=0;row[3]=2;
        for(let x=0;x<512;x++) row[x+4]=Math.round(50+14*Math.sin(x*7+y*3)+((x>70&&x<440)? 75*Math.pow(Math.sin(x*.12),10):0));
        socket.onmessage({data:row.buffer});
      }
      deliver({t:'host',ts:100,kind:'connected',line:'CONNECTED W1AW K6XAR 2300'});
      deliver({t:'data',ts:100,bytes:32,preview:'<script>alert("unsafe")</script>'});
      deliver({t:'signal',ts:100,name:'ACK'});
    });
    assert.equal(await page.locator('#received').innerText(),'24,832');
    assert.equal(await page.locator('#peer').innerText(),'K6XAR');
    assert(await page.locator('#led-ack').evaluate(el=>el.classList.contains('on')));
    assert.equal(await page.locator('#frame-level').innerText(),'9');
    assert.equal(await page.locator('#rx-peak').innerText(),'-1.5');
    assert.equal(await page.locator('#rx-clips').innerText(),'3');
    assert.equal(await page.locator('#tx-clips').innerText(),'2');
    assert.equal(await page.locator('#telemetry-drops').innerText(),'7');
    assert.equal(await page.locator('#passband-low').innerText(),'350.0');
    assert.equal(await page.locator('#passband-high').innerText(),'2,650.0');
    assert(await page.locator('#tx-clip').evaluate(el=>el.classList.contains('on')));
    await page.getByRole('tab',{name:'Diagnostics',exact:true}).click();
    assert.equal(await page.locator('#diag-version').innerText(),'test-1.2');
    assert.equal(await page.locator('#diag-backend').innerText(),'coreaudio');
    assert.equal(await page.locator('#diag-bins').innerText(),'512');
    assert.equal(await page.locator('#diag-rate').innerText(),'10.00');
    assert.equal(await page.locator('#diag-step').innerText(),'0.50');
    await page.getByRole('tab',{name:'Modem',exact:true}).click();
    assert.equal(await page.locator('#log script').count(),0);
    assert.match(await page.locator('#log').innerText(),/<script>/);
    assert.equal(await page.locator('#wf-empty').isVisible(),false);
    await page.screenshot({path:process.env.XARA_SCREENSHOT || '/tmp/xara-modem.png',fullPage:true});
    await page.getByRole('tab',{name:'Setup',exact:true}).click();
    assert(await page.locator('#panel-setup').isVisible());
    await page.locator('input[name=call]').fill('N0CALL');
    await page.locator('#set-save').click();
    assert((await page.evaluate(()=>commands)).some(c=>c.startsWith('SETTINGS SAVE\ncall N0CALL')));
    assert(!(await page.evaluate(()=>commands)).some(c=>c.includes('undefined')));
    await page.evaluate(()=>deliver({t:'settings_result',what:'save',ok:true,text:'Saved'}));
    assert((await page.evaluate(()=>commands)).includes('RESTART'));
    await page.screenshot({path:'/tmp/xara-setup.png',fullPage:true});
    await page.getByRole('tab',{name:'Modem',exact:true}).click();
    await page.locator('#log-clear').click();
    assert.equal(await page.locator('#log li').count(),0);
    assert(await page.locator('#log-empty').isVisible());
    const downloadPromise=page.waitForEvent('download');
    await page.locator('#log-export').click();
    assert.equal((await downloadPromise).suggestedFilename(),'xara-monitor.txt');
    await page.setViewportSize({width:390,height:844});
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
    await page.screenshot({path:'/tmp/xara-mobile.png',fullPage:true});
    await page.evaluate(()=>{clearInterval(statusTimer);socket.readyState=3});
    await page.waitForFunction(()=>document.getElementById('set-fields').disabled);
    assert.equal(await page.locator('#controls').isVisible(),false);
    assert.equal(await page.locator('#rx-peak').innerText(),'—');
    assert.equal(await page.locator('#telemetry-drops').innerText(),'—');
    await page.evaluate(()=>{socket.readyState=1;deliver({...sample,received_bytes:0})});
    await page.waitForFunction(()=>!document.getElementById('set-fields').disabled);
    assert.equal(await page.locator('#received').innerText(),'0');
    assert.equal(await page.locator('#set-save').isDisabled(),false);
    await page.evaluate(()=>{ clearInterval(statusTimer); deliver({...sample,bitrate:null,dropped:0,passband:[null,null],rx:{rms:-120,peak:-120,clip:0},tx:{rms:-120,peak:-120,clip:0}}); });
    assert.equal(await page.locator('#frame-level').innerText(),'—');
    assert.equal(await page.locator('#passband-low').innerText(),'—');
    assert.equal(await page.locator('#rx-clips').innerText(),'0');
    assert.equal(await page.locator('#rx-peak').innerText(),'≤ -120.0');
    assert(!(await page.locator('#tx-clip').evaluate(el=>el.classList.contains('on'))));
    assert.deepEqual(errors,[]);
    console.log('PASS: live instruments, safe payload display, waterfall, tabs, setup commands, export, mobile layout, disconnect/recovery, session counter reset');
  } finally { await browser.close(); }
})().catch(e=>{console.error(e);process.exit(1)});
