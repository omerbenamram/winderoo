import { useState } from 'preact/hooks'
import { useStore } from '../hooks/useStore'
import { languages } from '../i18n'
import { api } from '../api'
import { Icons } from './Icons'

export function Header() {
  const { status, translations: t, language, setLanguage, setPower, refresh } = useStore()
  const [showLangMenu, setShowLangMenu] = useState(false)
  const [showResetDialog, setShowResetDialog] = useState(false)

  const isOn = status?.winderEnabled === 1

  return (
    <>
      <header class="header">
        <div class="header-content">
          <h1 class="logo">
            <Icons.Watch />
            Winderoo
          </h1>

          <div class="header-actions">
            {/* Power toggle */}
            <div class="power-toggle">
              <span class="toggle-label">{isOn ? t.header.on : t.header.off}</span>
              <button
                class={`toggle ${isOn ? 'active' : ''}`}
                onClick={() => setPower(!isOn)}
                aria-label="Toggle power"
              />
            </div>

            {/* Reset button */}
            <button class="btn btn-outline btn-icon" onClick={() => setShowResetDialog(true)} title={t.header.reset}>
              <Icons.Bolt />
            </button>

            {/* Language selector */}
            <div class="lang-menu-wrapper">
              <button class="btn btn-outline btn-icon" onClick={() => setShowLangMenu(!showLangMenu)}>
                <Icons.Globe />
              </button>
              {showLangMenu && (
                <div class="lang-menu">
                  {languages.map((lang) => (
                    <button
                      key={lang.code}
                      class={`lang-option ${language === lang.code ? 'active' : ''}`}
                      onClick={() => {
                        setLanguage(lang.code)
                        setShowLangMenu(false)
                      }}
                    >
                      {lang.name}
                    </button>
                  ))}
                </div>
              )}
            </div>
          </div>
        </div>
      </header>

      {/* Reset dialog */}
      {showResetDialog && (
        <div class="dialog-overlay" onClick={() => setShowResetDialog(false)}>
          <div class="dialog" onClick={(e) => e.stopPropagation()}>
            <h2>{t.header.dialog.title}</h2>
            <p class="text-secondary mt-2">{t.header.dialog.subtitle}</p>
            
            <ul class="reset-steps">
              <li>{t.header.dialog.point1}</li>
              <li>{t.header.dialog.point2} <strong>Winderoo</strong></li>
              <li>{t.header.dialog.point3}</li>
              <li>{t.header.dialog.point4}</li>
            </ul>

            <p class="text-sm text-muted mt-3">{t.header.dialog.nowAvailable}</p>
            <code class="hostname">http://winderoo.local</code>

            <div class="dialog-actions">
              <button class="btn btn-outline" onClick={() => setShowResetDialog(false)}>
                {t.header.dialog.cancel}
              </button>
              <button
                class="btn btn-danger"
                onClick={async () => {
                  await api.reset()
                  setShowResetDialog(false)
                  setTimeout(refresh, 20000)
                }}
              >
                {t.header.dialog.confirmReset}
              </button>
            </div>
          </div>
        </div>
      )}

      <style>{`
        .header {
          background: var(--bg-secondary);
          border-bottom: 1px solid var(--border);
          padding: 1rem;
          position: sticky;
          top: 0;
          z-index: 100;
          backdrop-filter: blur(10px);
        }

        .header-content {
          max-width: 600px;
          margin: 0 auto;
          display: flex;
          align-items: center;
          justify-content: space-between;
        }

        .logo {
          font-size: 1.25rem;
          font-weight: 700;
          display: flex;
          align-items: center;
          gap: 0.5rem;
          background: linear-gradient(135deg, var(--accent), var(--accent-hover));
          -webkit-background-clip: text;
          -webkit-text-fill-color: transparent;
          background-clip: text;
        }

        .header-actions {
          display: flex;
          align-items: center;
          gap: 0.75rem;
        }

        .power-toggle {
          display: flex;
          align-items: center;
          gap: 0.5rem;
        }

        .toggle-label {
          font-size: 0.75rem;
          text-transform: uppercase;
          font-weight: 600;
          color: var(--text-secondary);
        }

        .lang-menu-wrapper {
          position: relative;
        }

        .lang-menu {
          position: absolute;
          top: 100%;
          right: 0;
          margin-top: 0.5rem;
          background: var(--bg-card);
          border: 1px solid var(--border);
          border-radius: var(--radius-md);
          overflow: hidden;
          min-width: 140px;
          box-shadow: var(--shadow);
        }

        .lang-option {
          width: 100%;
          padding: 0.75rem 1rem;
          text-align: left;
          transition: var(--transition);
        }

        .lang-option:hover {
          background: var(--bg-card-hover);
        }

        .lang-option.active {
          color: var(--accent);
          background: var(--accent-glow);
        }

        .dialog-overlay {
          position: fixed;
          inset: 0;
          background: rgba(0, 0, 0, 0.7);
          display: flex;
          align-items: center;
          justify-content: center;
          padding: 1rem;
          z-index: 200;
          backdrop-filter: blur(4px);
        }

        .dialog {
          background: var(--bg-card);
          border: 1px solid var(--border);
          border-radius: var(--radius-lg);
          padding: 1.5rem;
          max-width: 400px;
          width: 100%;
        }

        .reset-steps {
          margin: 1rem 0;
          padding-left: 1.25rem;
          color: var(--text-secondary);
          font-size: 0.875rem;
        }

        .reset-steps li {
          margin-bottom: 0.5rem;
        }

        .hostname {
          display: block;
          background: var(--bg-secondary);
          padding: 0.5rem 1rem;
          border-radius: var(--radius-sm);
          font-size: 0.875rem;
          margin-top: 0.5rem;
        }

        .dialog-actions {
          display: flex;
          gap: 0.75rem;
          margin-top: 1.5rem;
          justify-content: flex-end;
        }
      `}</style>
    </>
  )
}
