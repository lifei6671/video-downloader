window.addEventListener('load', () => {
  void import('./main').catch(() => {
    const screen = document.querySelector('.startup-screen');
    const message = screen?.querySelector('.startup-message');
    screen?.setAttribute('role', 'alert');
    screen?.removeAttribute('aria-live');
    if (message) message.textContent = '无法加载映取界面，请从托盘退出后重试。';
  });
}, { once: true });
