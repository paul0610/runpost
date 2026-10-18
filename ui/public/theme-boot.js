// Anti-destello (9.4): fija data-theme antes del primer render, desde el espejo de localStorage.
// Archivo externo y no <script> en línea: así la CSP de Tauri y la del modo navegador no necesitan excepciones.
(function () {
  var mode = 'system';
  try { mode = localStorage.getItem('runpost.theme') || 'system'; } catch (e) { /* almacenamiento bloqueado */ }
  var dark = mode === 'dark' || (mode !== 'light' && window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches);
  document.documentElement.setAttribute('data-theme', dark ? 'dark' : 'light');
  try {
    var size = localStorage.getItem('runpost.fontSize');
    if (size) document.documentElement.style.setProperty('--font-size', size + 'px');
  } catch (e) { /* ignorar */ }
})();
