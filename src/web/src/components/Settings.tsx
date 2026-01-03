import { useState, useEffect } from 'preact/hooks'
import { useStore } from '../hooks/useStore'
import { useClock, formatTime } from '../hooks/useClock'
import { api } from '../api'
import type { Direction, UpdatePayload } from '../types'
import { Icons } from './Icons'

const HOURS = Array.from({ length: 24 }, (_, i) => i.toString().padStart(2, '0'))
const MINUTES = ['00', '10', '20', '30', '40', '50']
const ALL_MINUTES = Array.from({ length: 60 }, (_, i) => i.toString().padStart(2, '0'))

const GMT_OFFSETS = [
  { value: -12, label: 'UTC-12:00' }, { value: -11, label: 'UTC-11:00' },
  { value: -10, label: 'UTC-10:00' }, { value: -9, label: 'UTC-09:00' },
  { value: -8, label: 'UTC-08:00' }, { value: -7, label: 'UTC-07:00' },
  { value: -6, label: 'UTC-06:00' }, { value: -5, label: 'UTC-05:00' },
  { value: -4, label: 'UTC-04:00' }, { value: -3, label: 'UTC-03:00' },
  { value: -2, label: 'UTC-02:00' }, { value: -1, label: 'UTC-01:00' },
  { value: 0, label: 'UTC±00:00' }, { value: 1, label: 'UTC+01:00' },
  { value: 2, label: 'UTC+02:00' }, { value: 3, label: 'UTC+03:00' },
  { value: 4, label: 'UTC+04:00' }, { value: 5, label: 'UTC+05:00' },
  { value: 5.5, label: 'UTC+05:30' }, { value: 6, label: 'UTC+06:00' },
  { value: 7, label: 'UTC+07:00' }, { value: 8, label: 'UTC+08:00' },
  { value: 9, label: 'UTC+09:00' }, { value: 10, label: 'UTC+10:00' },
  { value: 11, label: 'UTC+11:00' }, { value: 12, label: 'UTC+12:00' },
]

export function Settings() {
  const { status, loading, translations: t, refresh } = useStore()
  const [saving, setSaving] = useState(false)

  // Local form state
  const [direction, setDirection] = useState<Direction>('CW')
  const [rpd, setRpd] = useState(650)
  const [timerEnabled, setTimerEnabled] = useState(false)
  const [timerHour, setTimerHour] = useState('08')
  const [timerMinutes, setTimerMinutes] = useState('00')
  const [screenSleep, setScreenSleep] = useState(false)
  const [scheduleEnabled, setScheduleEnabled] = useState(false)
  const [scheduleStart, setScheduleStart] = useState({ hour: '08', minute: '00' })
  const [scheduleEnd, setScheduleEnd] = useState({ hour: '22', minute: '00' })
  const [windDuration, setWindDuration] = useState(180)
  const [pauseDuration, setPauseDuration] = useState(15)
  const [rotationTime, setRotationTime] = useState(8)
  const [gmtOffset, setGmtOffset] = useState(0)
  const [dst, setDst] = useState(false)

  // Sync from API status
  useEffect(() => {
    if (!status) return
    setDirection(status.direction)
    setRpd(status.rotationsPerDay)
    setTimerEnabled(status.timerEnabled === 1)
    setTimerHour(status.hour)
    setTimerMinutes(status.minutes)
    setScreenSleep(status.screenSleep)
    setScheduleEnabled(status.screenScheduleEnabled)
    if (status.screenScheduleStartTime) {
      const [h, m] = status.screenScheduleStartTime.split(':')
      setScheduleStart({ hour: h, minute: m })
    }
    if (status.screenScheduleEndTime) {
      const [h, m] = status.screenScheduleEndTime.split(':')
      setScheduleEnd({ hour: h, minute: m })
    }
    setWindDuration(status.customWindDuration || 180)
    setPauseDuration(status.customWindPauseDuration || 15)
    setRotationTime(status.customDurationInSecondsToCompleteOneRevolution)
    setGmtOffset(status.gmtOffset)
    setDst(status.dst)
  }, [status])

  const clockEpoch = useClock(status?.currentTimeEpoch || 0)

  if (loading || !status) {
    return (
      <div class="settings-loading">
        <div class="spinner" />
        <p>{t.settings.loading}</p>
      </div>
    )
  }

  if (status.winderEnabled === 0) {
    return null // Hidden when powered off
  }

  // Calculations
  const estimateDuration = () => {
    const totalTurning = rpd * rotationTime
    const restPeriods = totalTurning / windDuration
    const totalRest = restPeriods * pauseDuration
    const total = totalTurning + totalRest
    const hours = Math.floor(total / 3600)
    const mins = Math.floor((total % 3600) / 60)
    return { hours, mins }
  }

  const progress = (() => {
    if (status.status !== 'Winding') return 0
    const { startTimeEpoch, currentTimeEpoch, estimatedRoutineFinishEpoch } = status
    if (currentTimeEpoch >= estimatedRoutineFinishEpoch) return 100
    const diff = (currentTimeEpoch - startTimeEpoch) / (estimatedRoutineFinishEpoch - startTimeEpoch)
    return Math.max(0, Math.min(100, diff * 100))
  })()

  const wifiBars = status.db <= 30 ? 3 : status.db <= 60 ? 2 : 1

  const buildPayload = (action: 'START' | 'STOP'): UpdatePayload => ({
    action,
    rotationDirection: direction,
    tpd: rpd,
    hour: timerHour,
    minutes: timerMinutes,
    timerEnabled: timerEnabled ? 1 : 0,
    screenSleep,
    screenScheduleEnabled: scheduleEnabled,
    screenScheduleStartTime: `${scheduleStart.hour}:${scheduleStart.minute}`,
    screenScheduleEndTime: `${scheduleEnd.hour}:${scheduleEnd.minute}`,
    customWindDuration: windDuration,
    customWindPauseDuration: pauseDuration,
    customDurationInSecondsToCompleteOneRevolution: rotationTime,
    rtcGmtOffset: gmtOffset,
    rtcDST: dst,
  })

  const handleSave = async () => {
    setSaving(true)
    try {
      const action = status.status === 'Winding' ? 'START' : 'STOP'
      await api.updateSettings(buildPayload(action))
      await refresh()
    } finally {
      setSaving(false)
    }
  }

  const handleStart = async () => {
    setSaving(true)
    try {
      await api.updateSettings(buildPayload('START'))
      await refresh()
    } finally {
      setSaving(false)
    }
  }

  const handleStop = async () => {
    setSaving(true)
    try {
      await api.updateSettings(buildPayload('STOP'))
      await refresh()
    } finally {
      setSaving(false)
    }
  }

  const est = estimateDuration()
  const statusText = status.status === 'Winding' ? t.settings.winding : t.settings.stopped
  const directionLabels: Record<Direction, string> = {
    CW: t.settings.clockwise,
    CCW: t.settings.counterClockwise,
    BOTH: t.settings.both,
  }

  const toMinSec = (s: number) => ({ min: Math.floor(s / 60), sec: s % 60 })

  return (
    <main class="settings">
      {/* Status card */}
      <div class={`card status-card ${status.status.toLowerCase()}`}>
        <div class="flex items-center justify-between">
          <div class="flex items-center gap-2">
            <span class={`status-dot ${status.status.toLowerCase()}`} />
            <span class="font-medium">{t.settings.status}</span>
          </div>
          <div class="flex items-center gap-3">
            <span class="font-semibold">{statusText}</span>
            <Icons.Wifi bars={wifiBars} />
          </div>
        </div>
      </div>

      {/* Controls */}
      <div class="card">
        <div class="control-buttons">
          <button class="btn btn-success" onClick={handleStart} disabled={saving}>
            <Icons.Play /> Start
          </button>
          <button class="btn btn-danger" onClick={handleStop} disabled={saving}>
            <Icons.Stop /> Stop
          </button>
        </div>
      </div>

      {/* Progress (when winding) */}
      {status.status === 'Winding' && (
        <div class="card">
          <div class="flex items-center justify-between mb-2">
            <span>{progress < 5 ? t.settings.pleaseWait : t.settings.progress}</span>
            <span class="font-semibold">{Math.round(progress)}%</span>
          </div>
          <div class="progress">
            <div class="progress-bar" style={{ width: `${progress}%` }} />
          </div>
        </div>
      )}

      {/* Estimated duration */}
      <div class="card">
        <div class="setting-row">
          <span>{t.settings.estimatedDuration}</span>
          <span class="setting-value">{est.hours}h {est.mins}m</span>
        </div>
      </div>

      {/* Direction & RPD */}
      <div class="card">
        <div class="setting-section">
          <label>{t.settings.direction}</label>
          <div class="setting-value mb-2">{directionLabels[direction]}</div>
          <div class="direction-toggles">
            {(['CW', 'BOTH', 'CCW'] as Direction[]).map((d) => (
              <button
                key={d}
                class={`dir-btn ${direction === d ? 'active' : ''}`}
                onClick={() => setDirection(d)}
              >
                {d === 'CW' && <Icons.RotateCW />}
                {d === 'CCW' && <Icons.RotateCCW />}
                {d === 'BOTH' && <Icons.Sync />}
              </button>
            ))}
          </div>
        </div>

        <div class="setting-section mt-4">
          <label>{t.settings.rotationsPerDay}</label>
          <div class="setting-value">{rpd}</div>
          <input
            type="range"
            class="slider"
            min={100}
            max={960}
            step={20}
            value={rpd}
            onInput={(e) => setRpd(Number((e.target as HTMLInputElement).value))}
          />
        </div>

        <a
          href="https://watch-winder.store/watch-winding-table/"
          target="_blank"
          rel="noopener"
          class="btn btn-outline w-full mt-4"
        >
          <Icons.ExternalLink />
          {t.settings.findWindingParameters}
        </a>
      </div>

      {/* Timer */}
      <div class="card">
        <div class="setting-section">
          <div class="flex items-center justify-between">
            <label>{t.settings.cycleStart}</label>
            <button
              class={`toggle ${timerEnabled ? 'active' : ''}`}
              onClick={() => setTimerEnabled(!timerEnabled)}
            />
          </div>
          <div class="text-xs text-muted mt-1">
            {timerEnabled ? t.settings.enabled.toUpperCase() : t.settings.disabled.toUpperCase()}
          </div>

          {timerEnabled && (
            <div class="time-selects mt-3">
              <select class="select" value={timerHour} onChange={(e) => setTimerHour((e.target as HTMLSelectElement).value)}>
                {HOURS.map((h) => <option key={h} value={h}>{h}</option>)}
              </select>
              <span class="time-colon">:</span>
              <select class="select" value={timerMinutes} onChange={(e) => setTimerMinutes((e.target as HTMLSelectElement).value)}>
                {MINUTES.map((m) => <option key={m} value={m}>{m}</option>)}
              </select>
            </div>
          )}
        </div>
      </div>

      {/* Screen settings */}
      {status.screenEquipped && (
        <div class="card">
          <div class="setting-section">
            <div class="flex items-center justify-between">
              <label class="flex items-center gap-2">
                <Icons.Display />
                {t.settings.screen}
              </label>
              <button
                class={`toggle ${!screenSleep ? 'active' : ''}`}
                onClick={() => setScreenSleep(!screenSleep)}
              />
            </div>
            <div class="text-xs text-muted mt-1">
              {screenSleep ? t.settings.disabled.toUpperCase() : t.settings.enabled.toUpperCase()}
            </div>
          </div>

          <div class="setting-section mt-4">
            <div class="flex items-center justify-between">
              <label>{t.settings.screenSchedule}</label>
              <button
                class={`toggle ${scheduleEnabled ? 'active' : ''}`}
                onClick={() => setScheduleEnabled(!scheduleEnabled)}
              />
            </div>

            {scheduleEnabled && (
              <div class="schedule-times mt-3">
                <div class="schedule-row">
                  <span class="text-sm">{t.settings.turnOnScreenAt}</span>
                  <div class="time-selects">
                    <select class="select" value={scheduleStart.hour} onChange={(e) => setScheduleStart({ ...scheduleStart, hour: (e.target as HTMLSelectElement).value })}>
                      {HOURS.map((h) => <option key={h} value={h}>{h}</option>)}
                    </select>
                    <span class="time-colon">:</span>
                    <select class="select" value={scheduleStart.minute} onChange={(e) => setScheduleStart({ ...scheduleStart, minute: (e.target as HTMLSelectElement).value })}>
                      {ALL_MINUTES.map((m) => <option key={m} value={m}>{m}</option>)}
                    </select>
                  </div>
                </div>
                <div class="schedule-row">
                  <span class="text-sm">{t.settings.turnOffScreenAt}</span>
                  <div class="time-selects">
                    <select class="select" value={scheduleEnd.hour} onChange={(e) => setScheduleEnd({ ...scheduleEnd, hour: (e.target as HTMLSelectElement).value })}>
                      {HOURS.map((h) => <option key={h} value={h}>{h}</option>)}
                    </select>
                    <span class="time-colon">:</span>
                    <select class="select" value={scheduleEnd.minute} onChange={(e) => setScheduleEnd({ ...scheduleEnd, minute: (e.target as HTMLSelectElement).value })}>
                      {ALL_MINUTES.map((m) => <option key={m} value={m}>{m}</option>)}
                    </select>
                  </div>
                </div>
              </div>
            )}
          </div>
        </div>
      )}

      {/* Customization */}
      <div class="card">
        <h3 class="flex items-center gap-2 mb-4">
          <Icons.Settings />
          {t.settings.customize}
        </h3>

        <div class="setting-section">
          <label>{t.settings.rotationDurationTime}</label>
          <div class="setting-value">
            {toMinSec(windDuration).min}m {toMinSec(windDuration).sec}s
          </div>
          <input
            type="range"
            class="slider"
            min={100}
            max={960}
            step={10}
            value={windDuration}
            onInput={(e) => setWindDuration(Number((e.target as HTMLInputElement).value))}
          />
        </div>

        <div class="setting-section mt-4">
          <label>{t.settings.rotationPauseTime}</label>
          <div class="setting-value">
            {toMinSec(pauseDuration).min}m {toMinSec(pauseDuration).sec}s
          </div>
          <input
            type="range"
            class="slider"
            min={10}
            max={900}
            step={5}
            value={pauseDuration}
            onInput={(e) => setPauseDuration(Number((e.target as HTMLInputElement).value))}
          />
        </div>

        <div class="setting-section mt-4">
          <label>{t.settings.singleRotationTiming}</label>
          <div class="setting-value">{rotationTime} {t.settings.seconds}</div>
          <input
            type="range"
            class="slider"
            min={1}
            max={16}
            step={1}
            value={rotationTime}
            onInput={(e) => setRotationTime(Number((e.target as HTMLInputElement).value))}
          />
        </div>

        {/* Internal clock */}
        <div class="setting-section mt-4">
          <label class="flex items-center gap-2">
            <Icons.Clock />
            {t.settings.internalClock}
          </label>
          <div class="setting-value">{formatTime(clockEpoch)}</div>
          <p class="text-xs text-muted mt-2">{t.settings.internalClockBlurb}</p>

          <div class="mt-3">
            <label class="text-sm">{t.settings.utcOffset}</label>
            <select
              class="select w-full mt-1"
              value={gmtOffset}
              onChange={(e) => setGmtOffset(Number((e.target as HTMLSelectElement).value))}
            >
              {GMT_OFFSETS.map((o) => (
                <option key={o.value} value={o.value}>{o.label}</option>
              ))}
            </select>
          </div>

          <div class="flex items-center justify-between mt-3">
            <label>{t.settings.dst}</label>
            <button class={`toggle ${dst ? 'active' : ''}`} onClick={() => setDst(!dst)} />
          </div>
        </div>
      </div>

      {/* Save button */}
      <div class="card">
        <button class="btn btn-primary w-full" onClick={handleSave} disabled={saving}>
          {saving ? t.settings.saveInProgress : t.settings.save}
        </button>
      </div>

      <style>{`
        .settings {
          max-width: 600px;
          margin: 0 auto;
          padding: 1rem;
          display: flex;
          flex-direction: column;
          gap: 1rem;
          padding-bottom: 2rem;
        }

        .settings-loading {
          display: flex;
          flex-direction: column;
          align-items: center;
          justify-content: center;
          min-height: 60vh;
          gap: 1rem;
        }

        .spinner {
          width: 40px;
          height: 40px;
          border: 3px solid var(--border);
          border-top-color: var(--accent);
          border-radius: 50%;
          animation: spin 1s linear infinite;
        }

        .status-card.winding {
          border-color: var(--success);
          box-shadow: 0 0 20px var(--success-glow);
        }

        .status-card.stopped {
          border-color: var(--danger);
        }

        .control-buttons {
          display: grid;
          grid-template-columns: 1fr 1fr;
          gap: 1rem;
        }

        .setting-section {
          padding-bottom: 1rem;
          border-bottom: 1px solid var(--border);
        }

        .setting-section:last-child {
          border-bottom: none;
          padding-bottom: 0;
        }

        .setting-section label {
          font-weight: 500;
          color: var(--text-secondary);
          font-size: 0.875rem;
        }

        .setting-value {
          font-size: 1.25rem;
          font-weight: 600;
          color: var(--text-primary);
          margin: 0.25rem 0;
        }

        .setting-row {
          display: flex;
          justify-content: space-between;
          align-items: center;
        }

        .direction-toggles {
          display: flex;
          gap: 0.5rem;
        }

        .dir-btn {
          flex: 1;
          padding: 0.75rem;
          border: 2px solid var(--border);
          border-radius: var(--radius-md);
          transition: var(--transition);
          display: flex;
          align-items: center;
          justify-content: center;
        }

        .dir-btn:hover {
          border-color: var(--accent);
          color: var(--accent);
        }

        .dir-btn.active {
          background: var(--accent);
          border-color: var(--accent);
          color: white;
        }

        .time-selects {
          display: flex;
          align-items: center;
          gap: 0.5rem;
        }

        .time-selects .select {
          width: 80px;
        }

        .time-colon {
          font-size: 1.25rem;
          font-weight: 600;
        }

        .schedule-times {
          display: flex;
          flex-direction: column;
          gap: 1rem;
        }

        .schedule-row {
          display: flex;
          align-items: center;
          justify-content: space-between;
          flex-wrap: wrap;
          gap: 0.5rem;
        }

        @media (max-width: 400px) {
          .control-buttons {
            grid-template-columns: 1fr;
          }
          
          .schedule-row {
            flex-direction: column;
            align-items: flex-start;
          }
        }
      `}</style>
    </main>
  )
}
