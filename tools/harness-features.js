// Run with DATA=tools/test-data.json (made-up figures: the reading tests expect its readings of 12000 and
// 4500.5 kWh on 1 Nov 2026).
{showTab("overview");const out=[];const ok=(name,c,extra)=>out.push((c?'PASS ':'FAIL ')+name+(extra!==undefined?' — '+extra:''));
try{
  const n0=entries.length;
  // meter readings
  setMode('read');$('fRDate').value='2026-11-05';$('fElec').value='12050';$('fGas').value='4600.5';
  $('entryForm').dispatchEvent(new Event('submit',{cancelable:true}));
  const made=entries.filter(e=>e.reading);
  ok('reading adds two entries',entries.length===n0+2,entries.length-n0);
  ok('elec usage from reading',made.some(e=>e.elec===50&&e.from==='2026-11-02'&&e.to==='2026-11-05'),JSON.stringify(made.map(e=>[e.from,e.to,e.elec,e.gas])));
  ok('gas usage from reading',made.some(e=>e.gas===100));
  ok('reading msg ok',$('entryMsg').dataset.kind==='ok',$('entryMsg').textContent);
  $('fRDate').value='2026-11-06';$('fElec').value='12000';$('fGas').value='';
  $('entryForm').dispatchEvent(new Event('submit',{cancelable:true}));
  ok('lower reading refused',$('entryMsg').dataset.kind==='err'&&readings.length===4,$('entryMsg').textContent);
  $('fRDate').value='2026-11-05';$('fElec').value='12060';
  $('entryForm').dispatchEvent(new Event('submit',{cancelable:true}));
  ok('same-day reading refused',$('entryMsg').dataset.kind==='err',$('entryMsg').textContent);
  renderReadings();ok('only latest deletable',$('readingsBox').querySelectorAll('[data-act="rdel"]').length===2);
  const rid=latestReading('e').id;$('readingsBox').querySelector(`[data-rid="${rid}"] button`).click();$('readingsBox').querySelector('[data-act="rconfirm"]').click();
  ok('delete reading removes its entry',!entries.some(e=>e.reading===rid)&&readings.length===3);
  ok('reading entries have no edit buttons',[...$('entriesBox').querySelectorAll('tr')].filter(tr=>tr.textContent.includes('From readings')).every(tr=>!tr.querySelector('[data-act="edit"]')));
  // CSV
  const n1=entries.length;
  importCsv('from,to,electricity_kwh,gas_kwh\r\n2026-07-20,2026-07-21,1.5,2\r\n03/08/2026,03/08/2026,5,\r\nbad,row,1,1\r\n31/02/2026,31/02/2026,1,1\r\n');
  ok('csv adds valid rows',entries.length===n1+2,$('dataMsg').textContent);
  importCsv('from,to,electricity_kwh,gas_kwh\n2026-07-20,2026-07-21,1.5,2\n');
  ok('csv skips duplicates',entries.length===n1+2,$('dataMsg').textContent);
  const csv=toCsv(entries),rows=parseCsv(csv);
  ok('csv round trip',rows.length===entries.length+1&&rows[0].join()==='from,to,electricity_kwh,gas_kwh,source',rows.length);
  ok('csv quoting',parseCsv('a,"b,c","d""e"\n')[0].join('|')==='a|b,c|d"e');
  let threw='';try{importCsv('x,y\n1,2\n')}catch(e){threw=e.message}ok('csv without headers explains',/column names/.test(threw),threw);
  try{importJson('{nope')}catch(e){threw=e.message}ok('bad json explains',/valid JSON/.test(threw),threw);
  // new term: starts the day after this one ends, runs a year, and keeps the rates in force on its first day
  const ns=toS(toD(settingsDoc.termEnd)+DAY),ne=toS(toD(addMonths(ns,12))-DAY),carry=tariffFor(settingsDoc,ns).eRate;
  openNewTerm();ok('new term defaults',$('ntStart').value===ns&&$('ntEnd').value===ne,$('ntStart').value+' '+$('ntEnd').value);
  const keepR=['e','g'].map(latestReading).filter(Boolean).length,arch0=archive.length;
  $('ntGo').click();
  ok('new term archives',archive.length===arch0+1&&entries.length===0&&settingsDoc.termStart===ns,archive.length+' '+entries.length);
  ok('new term keeps latest readings',readings.length===keepR,readings.length);
  ok('new term carries tariff',settingsDoc.tariffs.length===1&&settingsDoc.tariffs[0].from===ns&&settingsDoc.tariffs[0].eRate===carry);
  ok('payload v2',JSON.parse(payload()).version===2&&Array.isArray(JSON.parse(payload()).archive));
  // view a past term
  setView(archive.length-1);ok('past term read-only',document.body.classList.contains('readonly')&&current().ro&&current().E.length>0);
  ok('banner offers way back',!!$('banner').querySelector('[data-act="back-term"]'));
  setView(-1);ok('back to current',!document.body.classList.contains('readonly'));
  // settings validation
  showTab('settings');$('sPlace').value='';$('sSeasonal').checked=true;$('settingsForm').dispatchEvent(new Event('submit',{cancelable:true}));
  ok('place required',$('settingsMsg').dataset.kind==='err',$('settingsMsg').textContent);
}catch(e){out.push('ERROR '+e.message+' '+(e.stack||'').split('\n')[1])}
const d=document.createElement('pre');d.style.cssText='position:fixed;inset:0;z-index:999;background:#fff;color:#000;font:12px monospace;padding:10px;margin:0;white-space:pre-wrap';d.textContent=out.join('\n');document.body.append(d);window.scrollTo(0,0)}
