import { useStore } from '../hooks/useStore'
import { Icons } from './Icons'

/** Get motor status info based on winder state */
function getMotorStatus(status: string, winderEnabled: number): { label: string; level: 'active' | 'idle' | 'off' } {
  if (winderEnabled === 0) return { label: 'Off', level: 'off' }
  if (status === 'Winding') return { label: 'Running', level: 'active' }
  if (status === 'Paused') return { label: 'Paused', level: 'idle' }
  return { label: 'Stopped', level: 'idle' }
}

/** Get WiFi signal strength label from RSSI dB (positive value, e.g. 50 means -50 dBm) */
function getSignalStrength(rssi: number): { label: string; level: 'excellent' | 'good' | 'fair' | 'weak' | 'none' } {
  // RSSI is stored as positive (abs value), so lower is better
  if (rssi <= 50) return { label: 'Excellent', level: 'excellent' }
  if (rssi <= 60) return { label: 'Good', level: 'good' }
  if (rssi <= 70) return { label: 'Fair', level: 'fair' }
  if (rssi <= 80) return { label: 'Weak', level: 'weak' }
  return { label: 'No Signal', level: 'none' }
}

/** Get WiFi bars count (0-4) from RSSI (positive value) */
function getWifiBars(rssi: number): number {
  // RSSI is stored as positive (abs value), so lower is better
  if (rssi <= 50) return 4
  if (rssi <= 60) return 3
  if (rssi <= 70) return 2
  if (rssi <= 80) return 1
  return 0
}

export function StatusBar() {
  const { status, translations: t } = useStore()

  if (!status) return null

  const signal = getSignalStrength(status.db)
  const wifiBars = getWifiBars(status.db)
  const motor = getMotorStatus(status.status, status.winderEnabled)

  return (
    <>
      <div class="status-bar">
        <div class="status-bar-content">
          {/* Motor Status */}
          <div class={`status-item motor-${motor.level}`}>
            <div class="status-icon">
              <Icons.Cog />
            </div>
            <div class="status-info">
              <span class="status-label">{t.statusBar?.motor || 'Motor'}</span>
              <span class="status-value">{motor.label}</span>
            </div>
          </div>

          {/* WiFi Signal */}
          <div class={`status-item ${signal.level}`}>
            <div class="status-icon">
              <WifiIcon bars={wifiBars} />
            </div>
            <div class="status-info">
              <span class="status-label">{t.statusBar?.wifi || 'WiFi'}</span>
              <span class="status-value">{signal.label} (-{status.db} dBm)</span>
            </div>
          </div>

          {/* Screen Status */}
          <div class={`status-item ${status.screenEquipped ? 'connected' : 'disconnected'}`}>
            <div class="status-icon">
              <Icons.Screen />
            </div>
            <div class="status-info">
              <span class="status-label">{t.statusBar?.screen || 'Screen'}</span>
              <span class="status-value">
                {status.screenEquipped
                  ? (t.statusBar?.connected || 'Connected')
                  : (t.statusBar?.notConnected || 'Not Connected')}
              </span>
            </div>
          </div>

          {/* API Version */}
          <div class="status-item version">
            <div class="status-icon">
              <Icons.Chip />
            </div>
            <div class="status-info">
              <span class="status-label">{t.statusBar?.firmware || 'Firmware'}</span>
              <span class="status-value">v{status.apiVersion}</span>
            </div>
          </div>
        </div>
      </div>

      <style>{`
        .status-bar {
          background: var(--bg-secondary);
          border-bottom: 1px solid var(--border);
          padding: 0.5rem 1rem;
        }

        .status-bar-content {
          max-width: 600px;
          margin: 0 auto;
          display: flex;
          align-items: center;
          justify-content: space-between;
          gap: 1rem;
          flex-wrap: wrap;
        }

        .status-item {
          display: flex;
          align-items: center;
          gap: 0.5rem;
          padding: 0.25rem 0.5rem;
          border-radius: var(--radius-sm);
          background: var(--bg-card);
          border: 1px solid var(--border);
          min-width: 0;
          flex: 1;
        }

        .status-icon {
          display: flex;
          align-items: center;
          justify-content: center;
          width: 24px;
          height: 24px;
          flex-shrink: 0;
        }

        .status-icon svg {
          width: 18px;
          height: 18px;
        }

        .status-info {
          display: flex;
          flex-direction: column;
          min-width: 0;
        }

        .status-label {
          font-size: 0.65rem;
          text-transform: uppercase;
          letter-spacing: 0.05em;
          color: var(--text-muted);
          font-weight: 600;
        }

        .status-value {
          font-size: 0.75rem;
          color: var(--text-secondary);
          white-space: nowrap;
          overflow: hidden;
          text-overflow: ellipsis;
        }

        /* WiFi signal levels */
        .status-item.excellent .status-icon svg {
          color: var(--success);
        }
        .status-item.excellent .status-value {
          color: var(--success);
        }

        .status-item.good .status-icon svg {
          color: var(--success);
        }
        .status-item.good .status-value {
          color: var(--success);
        }

        .status-item.fair .status-icon svg {
          color: var(--warning);
        }
        .status-item.fair .status-value {
          color: var(--warning);
        }

        .status-item.weak .status-icon svg {
          color: var(--danger);
        }
        .status-item.weak .status-value {
          color: var(--danger);
        }

        .status-item.none .status-icon svg {
          color: var(--text-muted);
        }

        /* Connection status */
        .status-item.connected .status-icon svg {
          color: var(--success);
        }
        .status-item.connected .status-value {
          color: var(--success);
        }

        .status-item.disconnected .status-icon svg {
          color: var(--text-muted);
        }
        .status-item.disconnected .status-value {
          color: var(--text-muted);
        }

        /* Version */
        .status-item.version .status-icon svg {
          color: var(--accent);
        }

        /* Motor status */
        .status-item.motor-active .status-icon svg {
          color: var(--success);
          animation: spin 2s linear infinite;
        }
        .status-item.motor-active .status-value {
          color: var(--success);
        }

        .status-item.motor-idle .status-icon svg {
          color: var(--warning);
        }
        .status-item.motor-idle .status-value {
          color: var(--warning);
        }

        .status-item.motor-off .status-icon svg {
          color: var(--text-muted);
        }
        .status-item.motor-off .status-value {
          color: var(--text-muted);
        }

        @keyframes spin {
          from { transform: rotate(0deg); }
          to { transform: rotate(360deg); }
        }

        @media (max-width: 480px) {
          .status-bar-content {
            gap: 0.5rem;
          }
          .status-item {
            padding: 0.25rem 0.375rem;
          }
          .status-label {
            font-size: 0.6rem;
          }
          .status-value {
            font-size: 0.7rem;
          }
        }
      `}</style>
    </>
  )
}

/** WiFi icon with animated bars based on signal strength */
function WifiIcon({ bars }: { bars: number }) {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <path d="M12 20h.01" opacity={bars >= 0 ? 1 : 0.2} />
      <path d="M8.5 16.429a5 5 0 0 1 7 0" opacity={bars >= 1 ? 1 : 0.2} />
      <path d="M5 12.859a10 10 0 0 1 14 0" opacity={bars >= 2 ? 1 : 0.2} />
      <path d="M1.5 9.11a15 15 0 0 1 21 0" opacity={bars >= 3 ? 1 : 0.2} />
    </svg>
  )
}
