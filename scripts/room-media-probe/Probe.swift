// macOS WKWebView fixture. Synthetic media only; never requests OS capture.
import AppKit
import WebKit
final class Probe: NSObject, WKNavigationDelegate, WKScriptMessageHandler {
 var view: WKWebView!
 func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) { print(message.body); fflush(stdout);if let s=message.body as? String,s.contains("\"event\":\"done\""){exit(0)} }
 func webView(_ view: WKWebView, didFinish navigation: WKNavigation!) {let script=try! String(contentsOfFile:CommandLine.arguments[1],encoding:.utf8);view.evaluateJavaScript(script){_,e in if let e=e{print(e);fflush(stdout);exit(2)}}}
}
let activity=ProcessInfo.processInfo.beginActivity(options:[.userInitiated,.latencyCritical],reason:"Synthetic media benchmark");let app=NSApplication.shared;app.setActivationPolicy(.accessory)
let probe=Probe(), config=WKWebViewConfiguration();config.websiteDataStore = .nonPersistent();config.mediaTypesRequiringUserActionForPlayback=[];config.userContentController.add(probe,name:"result")
probe.view=WKWebView(frame:NSRect(x:0,y:0,width:1280,height:800),configuration:config);probe.view.navigationDelegate=probe;let window=NSWindow(contentRect:NSRect(x:80,y:80,width:1000,height:700),styleMask:[.titled,.closable],backing:.buffered,defer:false);window.title="Otto synthetic media probe";window.contentView=probe.view;window.orderFrontRegardless();probe.view.loadHTMLString("<!doctype html><title>Synthetic room benchmark</title>",baseURL:URL(string:"http://localhost:47891/"));DispatchQueue.main.asyncAfter(deadline:.now()+130){print("TIMEOUT");fflush(stdout);exit(3)};let tick=Timer.scheduledTimer(withTimeInterval:1.0/15.0,repeats:true){_ in probe.view.evaluateJavaScript("window.__nativeTick?.();void 0",completionHandler:nil)};app.run()
