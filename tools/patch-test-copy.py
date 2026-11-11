# Patch a TEST COPY of app.html (never the real one) so a headless screenshot shows real figures:
# inline the weather and the data so the first paint is synchronous, and seed views from the URL hash
# (#m=<month index>, #tab=<overview|entries|settings>, #open=<css selector to click>).
# usage: patch-test-copy.py <app.html copy> <weather.json> <data.json> [harness.js]
import sys
p, weather, data = sys.argv[1], open(sys.argv[2]).read(), open(sys.argv[3]).read()
harness = open(sys.argv[4]).read() if len(sys.argv) > 4 else ''
s = open(p).read()
def rep(old, new):
    global s
    assert s.count(old) == 1, old
    s = s.replace(old, new)
rep('let weather=null;', 'let weather=' + weather + ';')
rep('let chartMonth=null;', r'let chartMonth=/m=(\d+)/.test(location.hash)?+location.hash.match(/m=(\d+)/)[1]:null;')
sync = ("{const D=" + data + ";applyData(D);if(!settingsDoc)enterSetup();"
        "else{fillSettings();document.body.classList.remove('loading');render()}}")
hook = (r"{const _t=location.hash.match(/tab=(\w+)/);if(_t)showTab(_t[1])}"
        r"{const _o=location.hash.match(/open=([^&]+)/);if(_o)document.querySelector(decodeURIComponent(_o[1])).click()}")
rep("(async()=>{\n  let o=null,r;", sync + harness + hook + "\n(async()=>{\n  let o=null,r;")
open(p, 'w').write(s)
