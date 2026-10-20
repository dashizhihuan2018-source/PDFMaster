const {test}=require('node:test'),assert=require('node:assert/strict');const {password}=require('../qpdf.cjs');
test('password admission rejects argument injection and excessive UTF-8',()=>{assert.equal(password('synthetic-password'),'synthetic-password');for(const p of ['short','long\n--bad\nargument','a'.repeat(128),'中'.repeat(43)])assert.throws(()=>password(p));});
