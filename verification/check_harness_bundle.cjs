// 只解析桌面 profile 的插件元数据，不启动 Harness 或读取会话正文。
const { pathToFileURL } = require('node:url');
const { join } = require('node:path');
(async () => {
  const runtime = process.argv[2];
  const profile = process.argv[3];
  const { loadProfileDirectory, composeEntries, createRuntimeResolution, PluginPackages } = await import(pathToFileURL(join(runtime, 'node_modules/@deepseek-ai/dsh-app-boot/lib/index.js')));
  const result = loadProfileDirectory('dsh', profile, join(runtime, 'node_modules/@deepseek-ai/dsh/package.json'));
  const own = '@dsh-external/agmanage-observer';
  console.log(JSON.stringify({ loaded: result.layers.some(layer => layer.packageName === own), skipped: result.skippedBundles.filter(bundle => bundle.packageName === own) }));
  const rows = composeEntries([...result.layers.map(layer => layer.patches), result.patches]);
  function visit(entries, parents = []) {
    for (const entry of entries) {
      if (/session-query|dsh-session$|agmanage/.test(entry.name || '')) console.log(JSON.stringify({ name: entry.name, disabled: entry.disabled, parents }));
      if (entry.group && Array.isArray(entry.config)) visit(entry.config, [...parents, entry.id]);
    }
  }
  visit(rows);
  const { Context } = await import(pathToFileURL(join(runtime, 'node_modules/@deepseek-ai/cordis/lib/index.js')));
  const ctx = new Context();
  ctx.baseUrl = pathToFileURL(profile + '/').href;
  ctx.provide('profileContext', { name: 'desktop' });
  console.log(JSON.stringify({ propertyVisible: ctx.profileContext?.name === 'desktop', getVisible: ctx.get('profileContext')?.name === 'desktop' }));
  if (process.argv[4]) {
    ctx.provide('sessionQuery', { listSessions: async () => [], readTitleSnapshots: async () => [] });
    ctx.provide('sessions', { get: () => undefined });
    const { Loader } = await import(pathToFileURL(join(runtime, 'node_modules/@deepseek-ai/cordis-plugin-loader/lib/index.js')));
    await ctx.plugin(Loader);
    const resolution = await createRuntimeResolution({ installAnchor: join(runtime, 'node_modules/@deepseek-ai/dsh/package.json'), profile: result });
    await ctx.plugin(PluginPackages, { resolution });
    const afterResolution = loadProfileDirectory('dsh', profile, join(runtime, 'node_modules/@deepseek-ai/dsh/package.json'));
    console.log(JSON.stringify({ afterRuntimeResolution: afterResolution.layers.some(layer => layer.packageName === own), skipped: afterResolution.skippedBundles.filter(bundle => bundle.packageName === own) }));
    await ctx.loader.root.create({ id: 'fixture-observer', name: own, config: {} });
    await ctx.loader.await();
    await new Promise(resolve => setTimeout(resolve, 500));
    console.log(JSON.stringify({ fixtureFiberState: ctx.loader.resolve('fixture-observer').fiber?.state }));
  }
  await ctx.fiber.dispose();
})().catch(error => { console.error(error.message); process.exitCode = 1; });
