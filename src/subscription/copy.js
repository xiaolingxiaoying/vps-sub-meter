// One copy action is shared by subscription links and native node links.
function copyUrl(btn, text) {
  if (!btn.dataset.copyLabel) btn.dataset.copyLabel = btn.textContent;
  var attempt = (btn.copyAttempt || 0) + 1;
  btn.copyAttempt = attempt;
  clearTimeout(btn.copyTimer);

  function finish(copied) {
    if (btn.copyAttempt !== attempt) return;
    btn.textContent = copied ? '已复制 ✓' : '复制失败';
    btn.classList.toggle('btn-success', copied);
    var status = document.getElementById('copy-status');
    if (status) {
      status.hidden = false;
      status.textContent = copied ? '链接已复制。' : '复制失败，请手动选择并复制链接。';
    }
    btn.copyTimer = setTimeout(function() {
      btn.textContent = btn.dataset.copyLabel;
      btn.classList.remove('btn-success');
    }, 2000);
  }

  function fallback() {
    if (btn.copyAttempt !== attempt) return;
    var previousFocus = document.activeElement;
    var ta = document.createElement('textarea');
    ta.value = text;
    ta.style.position = 'fixed';
    ta.style.left = '-9999px';
    ta.style.top = '-9999px';
    document.body.appendChild(ta);
    var copied = false;
    try {
      ta.focus();
      ta.select();
      copied = document.execCommand('copy');
    } catch (e) {
      // Older browsers may reject clipboard access even through execCommand.
    } finally {
      document.body.removeChild(ta);
      if (previousFocus && previousFocus.focus) previousFocus.focus();
    }
    finish(copied);
  }

  if (navigator.clipboard && navigator.clipboard.writeText) {
    navigator.clipboard.writeText(text).then(function() { finish(true); }, fallback);
  } else {
    fallback();
  }
}
