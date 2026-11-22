// Read native browser values without replacing navigator or the network UA.
(() => {
  const data = navigator.userAgentData;
  const user_agent = navigator.userAgent;
  const environment = {platform: navigator.platform, language: navigator.language,
    timezone: Intl.DateTimeFormat().resolvedOptions().timeZone || '',
    screen_width: screen.width, screen_height: screen.height,
    hardware_concurrency: navigator.hardwareConcurrency || null, device_memory: navigator.deviceMemory || null};
  const quote = value => JSON.stringify(value);
  const brands = list => list.map(item => `${quote(item.brand)};v=${quote(item.version)}`).join(', ');
  const finish = extra => {
    const hints = {};
    if (data) {
      if (Array.isArray(data.brands) && data.brands.length) hints['sec-ch-ua'] = brands(data.brands);
      hints['sec-ch-ua-mobile'] = data.mobile ? '?1' : '?0';
      if (data.platform) hints['sec-ch-ua-platform'] = quote(data.platform);
      for (const [field, header] of [['platformVersion', 'platform-version'], ['architecture', 'arch'], ['bitness', 'bitness']]) {
        if (extra[field]) hints[`sec-ch-ua-${header}`] = quote(extra[field]);
      }
      if (Array.isArray(extra.fullVersionList) && extra.fullVersionList.length) hints['sec-ch-ua-full-version-list'] = brands(extra.fullVersionList);
    }
    window.__yingquBrowserIdentity = Object.freeze({user_agent, client_hints: hints, environment});
  };
  window.__yingquBrowserIdentity = null;
  if (data?.getHighEntropyValues) {
    data.getHighEntropyValues(['platformVersion', 'architecture', 'bitness', 'fullVersionList']).then(finish).catch(() => finish({}));
  } else finish({});
})();
