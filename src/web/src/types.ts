export interface Status {
  batteryLevel: number | null
  db: number
  status: 'Winding' | 'Stopped' | 'Paused'
  rotationsPerDay: number
  direction: 'CW' | 'CCW' | 'BOTH'
  hour: string
  minutes: string
  startTimeEpoch: number
  currentTimeEpoch: number
  estimatedRoutineFinishEpoch: number
  winderEnabled: number
  timerEnabled: number
  screenSleep: boolean
  screenScheduleEnabled: boolean
  screenScheduleStartTime: string
  screenScheduleEndTime: string
  screenEquipped: boolean
  customWindDuration: number
  customWindPauseDuration: number
  customDurationInSecondsToCompleteOneRevolution: number
  gmtOffset: number
  apiVersion: string
  dst: boolean
}

export interface UpdatePayload {
  action: 'START' | 'STOP'
  rotationDirection: 'CW' | 'CCW' | 'BOTH'
  tpd: number
  hour: string
  minutes: string
  timerEnabled: number
  screenSleep: boolean
  screenScheduleEnabled: boolean
  screenScheduleStartTime: string
  screenScheduleEndTime: string
  customWindDuration: number
  customWindPauseDuration: number
  customDurationInSecondsToCompleteOneRevolution: number
  rtcGmtOffset: number
  rtcDST: boolean
}

export type Direction = 'CW' | 'CCW' | 'BOTH'
export type Language = 'en-US' | 'de-DE' | 'es-ES' | 'fr-FR' | 'pt-BR'
