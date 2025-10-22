package logger

import (
	"fmt"
	"io"
	"os"
	"strings"
	"sync"
	"time"
)

const (
	colorGreen   = "\033[32m"
	colorBlue    = "\033[34m"
	colorMagenta = "\033[35m"
	colorYellow  = "\033[33m"
	colorRed     = "\033[31m"
	colorReset   = "\033[0m"
)

type logLevel int

const (
	DEBUG logLevel = iota
	INFO
	WARN
	ERROR
	CRITICAL
)

var levelNames = []string{"DEBUG", "INFO", "WARNING", "ERROR", "CRITICAL"}
var levelColors = []string{colorBlue, colorGreen, colorYellow, colorRed, colorMagenta}

var levelMap = map[string]logLevel{
	"DEBUG":    DEBUG,
	"INFO":     INFO,
	"WARNING":  WARN,
	"WARN":     WARN,
	"ERROR":    ERROR,
	"CRITICAL": CRITICAL,
}

type Logger struct {
	name   string
	level  logLevel
	mu     sync.Mutex
	writer io.Writer
}

func NewLogger(name string, level string, writer io.Writer) *Logger {
	if writer == nil {
		writer = os.Stdout
	}
	lvl, ok := levelMap[strings.ToUpper(level)]
	if !ok {
		lvl = INFO
	}
	return &Logger{
		name:   name,
		level:  lvl,
		writer: writer,
	}
}

func (l *Logger) logf(level logLevel, format string, args ...interface{}) {
	if level < l.level {
		return
	}
	msg := fmt.Sprintf(format, args...)

	now := time.Now().Format("2006-01-02 15:04:05.000")
	ts := fmt.Sprintf("%s[%s]%s", colorGreen, now, colorReset)

	lvlColor := levelColors[level]
	lvlStr := fmt.Sprintf("%s%s%s", lvlColor, levelNames[level], colorReset)

	name := fmt.Sprintf("[%s%s%s]", colorMagenta, l.name, colorReset)

	line := fmt.Sprintf("%s[%s]%s %s\n", ts, lvlStr, name, msg)

	l.mu.Lock()
	defer l.mu.Unlock()
	fmt.Fprint(l.writer, line)
}

func (l *Logger) Debugf(format string, args ...interface{})    { l.logf(DEBUG, format, args...) }
func (l *Logger) Infof(format string, args ...interface{})     { l.logf(INFO, format, args...) }
func (l *Logger) Warnf(format string, args ...interface{})     { l.logf(WARN, format, args...) }
func (l *Logger) Errorf(format string, args ...interface{})    { l.logf(ERROR, format, args...) }
func (l *Logger) Criticalf(format string, args ...interface{}) { l.logf(CRITICAL, format, args...) }
func (l *Logger) Exceptionf(format string, args ...interface{}) {
	l.logf(ERROR, "[EXCEPTION] "+format, args...)
}
