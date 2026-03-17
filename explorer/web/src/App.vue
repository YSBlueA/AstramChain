<template>
  <div id="app">
    <!-- Noise overlay -->
    <div class="noise-overlay"></div>

    <nav class="navbar">
      <div class="nav-inner">
        <router-link to="/" class="nav-logo">
          <span class="logo-icon">◈</span>
          <span>Astram <span class="accent">Explorer</span></span>
        </router-link>
        <ul class="nav-links">
          <li><router-link to="/">Home</router-link></li>
          <li><router-link to="/blocks">Blocks</router-link></li>
          <li><router-link to="/transactions">Transactions</router-link></li>
          <li><router-link to="/richlist">Rich List</router-link></li>
          <li><router-link to="/node" class="nav-status-link">Node Status</router-link></li>
        </ul>
        <div class="nav-right">
          <span class="live-dot"></span>
          <span class="live-text">LIVE</span>
        </div>
      </div>
    </nav>

    <main class="main-content">
      <router-view />
    </main>

    <footer class="footer">
      <div class="footer-inner">
        <span class="footer-logo">◈ Astram Explorer</span>
        <span class="footer-copy">© 2025 Astram Network — Blockchain made simple</span>
      </div>
    </footer>
  </div>
</template>

<script>
export default { name: "App" };
</script>

<style>
/* ── Global Reset & Variables ── */
:root {
  --bg:       #080c14;
  --bg2:      #0d1420;
  --surface:  #111827;
  --surface2: #1a2535;
  --border:   #1f2d3d;
  --border2:  #263347;
  --accent:   #3b82f6;
  --accent2:  #8b5cf6;
  --green:    #10b981;
  --yellow:   #f59e0b;
  --red:      #ef4444;
  --text:     #e2e8f0;
  --text2:    #94a3b8;
  --muted:    #475569;
  --mono:     "Courier New", "Consolas", monospace;
  --radius:   10px;
  --radius-lg:16px;
}

*, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }

html { scroll-behavior: smooth; }

body {
  background: var(--bg);
  color: var(--text);
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
  font-size: 14px;
  line-height: 1.6;
  overflow-x: hidden;
}

a { color: var(--accent); text-decoration: none; }
a:hover { color: #60a5fa; }

/* Scrollbar */
::-webkit-scrollbar { width: 6px; height: 6px; }
::-webkit-scrollbar-track { background: var(--bg2); }
::-webkit-scrollbar-thumb { background: var(--border2); border-radius: 3px; }
::-webkit-scrollbar-thumb:hover { background: var(--muted); }
</style>

<style scoped>
.noise-overlay {
  position: fixed;
  inset: 0;
  background: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='300' height='300'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.65' numOctaves='3' stitchTiles='stitch'/%3E%3C/filter%3E%3Crect width='300' height='300' filter='url(%23n)' opacity='0.025'/%3E%3C/svg%3E");
  pointer-events: none;
  z-index: 0;
}

#app {
  display: flex;
  flex-direction: column;
  min-height: 100vh;
  position: relative;
  z-index: 1;
}

/* ── Navbar ── */
.navbar {
  position: sticky;
  top: 0;
  z-index: 200;
  background: rgba(8, 12, 20, 0.88);
  backdrop-filter: blur(14px);
  -webkit-backdrop-filter: blur(14px);
  border-bottom: 1px solid var(--border);
  height: 60px;
}

.nav-inner {
  max-width: 1280px;
  margin: 0 auto;
  padding: 0 2rem;
  height: 100%;
  display: flex;
  align-items: center;
  gap: 2rem;
}

.nav-logo {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 16px;
  font-weight: 700;
  color: var(--text);
  white-space: nowrap;
}

.nav-logo:hover { color: var(--text); }

.logo-icon {
  color: var(--accent);
  font-size: 20px;
}

.accent { color: var(--accent); }

.nav-links {
  display: flex;
  list-style: none;
  gap: 0;
  flex: 1;
}

.nav-links li a {
  color: var(--text2);
  font-size: 13.5px;
  padding: 0 1rem;
  height: 60px;
  display: flex;
  align-items: center;
  transition: color 0.2s;
  border-bottom: 2px solid transparent;
}

.nav-links li a:hover {
  color: var(--text);
}

.nav-links li a.router-link-active {
  color: var(--accent);
  border-bottom-color: var(--accent);
}

.nav-right {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-left: auto;
}

.live-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--green);
  box-shadow: 0 0 6px var(--green);
  animation: pulse-dot 2s infinite;
}

@keyframes pulse-dot {
  0%, 100% { opacity: 1; box-shadow: 0 0 6px var(--green); }
  50%       { opacity: 0.6; box-shadow: 0 0 2px var(--green); }
}

.live-text {
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 1px;
  color: var(--green);
}

/* ── Main ── */
.main-content {
  flex: 1;
  max-width: 1280px;
  width: 100%;
  margin: 0 auto;
  padding: 2rem;
}

/* ── Footer ── */
.footer {
  border-top: 1px solid var(--border);
  background: var(--bg2);
  padding: 1.25rem 2rem;
  margin-top: 3rem;
}

.footer-inner {
  max-width: 1280px;
  margin: 0 auto;
  display: flex;
  justify-content: space-between;
  align-items: center;
  flex-wrap: wrap;
  gap: 0.5rem;
}

.footer-logo {
  font-weight: 700;
  color: var(--accent);
  font-size: 13px;
}

.footer-copy {
  color: var(--muted);
  font-size: 12px;
}

@media (max-width: 768px) {
  .nav-inner { padding: 0 1rem; gap: 1rem; }
  .nav-links {
    display: none;
  }
  .main-content { padding: 1rem; }
  .footer-inner { flex-direction: column; align-items: flex-start; }
}
</style>
