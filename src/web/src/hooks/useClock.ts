import { useState, useEffect, useRef } from 'preact/hooks'

export function useClock(initialEpoch: number) {
  const [epoch, setEpoch] = useState(initialEpoch)
  const intervalRef = useRef<number | null>(null)

  useEffect(() => {
    setEpoch(initialEpoch)
    
    if (intervalRef.current) {
      clearInterval(intervalRef.current)
    }

    intervalRef.current = window.setInterval(() => {
      setEpoch((e) => e + 1)
    }, 1000)

    return () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current)
      }
    }
  }, [initialEpoch])

  return epoch
}

export function formatTime(epochSeconds: number): string {
  const date = new Date(epochSeconds * 1000)
  return date.toLocaleTimeString('en-US', {
    timeZone: 'UTC',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  })
}
