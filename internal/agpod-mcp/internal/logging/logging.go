package logging

import (
	"fmt"
	"io"
	"log/slog"
	"os"
	"strings"
)

const EnvLogLevel = "AGPOD_MEMO_LOG_LEVEL"

// New constructs a slog text logger writing to out at the level resolved from env.
func New(out io.Writer) (*slog.Logger, error) {
	level, err := levelFromEnv()
	if err != nil {
		return nil, err
	}
	handler := slog.NewTextHandler(out, &slog.HandlerOptions{Level: level})
	return slog.New(handler), nil
}

func ConfigureDefault(out io.Writer) (*slog.Logger, error) {
	logger, err := New(out)
	if err != nil {
		return nil, err
	}
	slog.SetDefault(logger)
	return logger, nil
}

func levelFromEnv() (slog.Level, error) {
	raw := strings.TrimSpace(os.Getenv(EnvLogLevel))
	if raw == "" {
		return slog.LevelInfo, nil
	}
	switch strings.ToLower(raw) {
	case "debug":
		return slog.LevelDebug, nil
	case "info":
		return slog.LevelInfo, nil
	case "warn", "warning":
		return slog.LevelWarn, nil
	case "error":
		return slog.LevelError, nil
	default:
		return 0, fmt.Errorf("%s must be one of debug|info|warn|error", EnvLogLevel)
	}
}
