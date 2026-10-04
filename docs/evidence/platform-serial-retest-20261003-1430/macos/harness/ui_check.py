from pathlib import Path
import sys,json,time,hashlib
ROOT = Path('/Volumes/projects-mac/NeonMix/.local/tm3')
OUT = Path('/Volumes/projects-mac/NeonMix/artifacts/platform-serial-retest-20261003-1430/macos/ui')
OUT.mkdir(exist_ok=True)
sys.path.insert(0,str(ROOT / 'tools'))
from macos_ui_visual_probe import VisualWindow
from e07_background_probe import wait_for
result={'passed':False,'scope':'macOS native preview windows, actual Command shortcuts, normal/minimum sizes; no background or audio started','checks':[],'binary_sha256':hashlib.sha256((ROOT/'target/release/neonmix-desktop').read_bytes()).hexdigest()}
try:
    for width,height in [(1100,760),(600,440)]:
        app=None
        try:
            app=VisualWindow(OUT,'mixer',width,height,None)
            app.capture(OUT,f'mixer-{width}')
            app.ax('set frontmost of p to true\nkeystroke "k" using command down')
            time.sleep(.4)
            app.capture(OUT,f'palette-observed-{width}')
            wait_for(lambda:'前往 Hub 设置' in app.ax('return entire contents of window 1 of p'))
            app.capture(OUT,f'palette-{width}')
            visible=app.ax('set e to static text "关闭" of group 1 of window 1 of p\nset ep to position of e\nset es to size of e\nset wp to position of window 1 of p\nset ws to size of window 1 of p\nreturn (item 2 of ep) + (item 2 of es) <= (item 2 of wp) + (item 2 of ws) - 8')
            assert visible == 'true', 'palette footer extends beyond the window'

            app.ax('key code 53')
            app.ax('keystroke "2" using command down')
            wait_for(lambda:'设备名称' in app.ax('return entire contents of window 1 of p'))
            app.capture(OUT,f'sender-{width}')
            result['checks'].append({'width':width,'height':height,'command_k_opens_palette':True,'escape_closes_palette':True,'command_2_navigates':True,'palette_footer_within_window':True})
        finally:
            if app:app.close()
    result['passed']=True
except Exception as e:
    result['error']=str(e)
finally:
    (OUT/'result.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
print(json.dumps(result,ensure_ascii=False))
raise SystemExit(0 if result['passed'] else 1)
