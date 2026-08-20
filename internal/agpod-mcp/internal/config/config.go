package config

import (
	"errors"
	"fmt"
	"os"
	"strings"
)

const (
	EnvHonchoAPIKey      = "HONCHO_API_KEY"
	EnvHonchoBaseURL     = "HONCHO_BASE_URL"
	EnvHonchoWorkspaceID = "HONCHO_WORKSPACE_ID"
	EnvPeerID            = "AGPOD_MEMO_PEER_ID"
	EnvRepoRoot          = "AGPOD_MEMO_REPO_ROOT"
	EnvReadonly          = "AGPOD_MEMO_READONLY"
	EnvListen            = "AGPOD_MEMO_LISTEN"
	EnvToken             = "AGPOD_MEMO_TOKEN"

	DefaultBaseURL = "https://api.honcho.dev"
	DefaultPeerID  = "agpod-agent"
)

type Config struct {
	HonchoAPIKey      string
	HonchoBaseURL     string
	HonchoWorkspaceID string
	PeerID            string
	RepoRoot          string
	// Readonly disables note and forget. ask_note is always exposed.
	Readonly bool
	// Listen, if set, serves Streamable HTTP instead of stdio.
	Listen string
	// Token, if set, requires Authorization: Bearer on HTTP.
	Token string
}

func FromEnv() (Config, error) {
	cfg := Config{
		HonchoAPIKey:      strings.TrimSpace(os.Getenv(EnvHonchoAPIKey)),
		HonchoBaseURL:     strings.TrimSpace(os.Getenv(EnvHonchoBaseURL)),
		HonchoWorkspaceID: strings.TrimSpace(os.Getenv(EnvHonchoWorkspaceID)),
		PeerID:            strings.TrimSpace(os.Getenv(EnvPeerID)),
		RepoRoot:          strings.TrimSpace(os.Getenv(EnvRepoRoot)),
		Listen:            strings.TrimSpace(os.Getenv(EnvListen)),
		Token:             strings.TrimSpace(os.Getenv(EnvToken)),
	}
	if cfg.HonchoBaseURL == "" {
		cfg.HonchoBaseURL = DefaultBaseURL
	}
	if cfg.PeerID == "" {
		cfg.PeerID = DefaultPeerID
	}
	if cfg.RepoRoot == "" {
		wd, err := os.Getwd()
		if err != nil {
			return Config{}, fmt.Errorf("resolve repo root from cwd: %w", err)
		}
		cfg.RepoRoot = wd
	}
	cfg.Readonly = parseBoolEnv(EnvReadonly)
	if err := cfg.Validate(); err != nil {
		return Config{}, err
	}
	return cfg, nil
}

func parseBoolEnv(name string) bool {
	switch strings.ToLower(strings.TrimSpace(os.Getenv(name))) {
	case "1", "true", "yes", "on":
		return true
	}
	return false
}

func (c Config) Validate() error {
	if c.HonchoAPIKey == "" {
		return fmt.Errorf("%s must be set", EnvHonchoAPIKey)
	}
	if c.HonchoWorkspaceID == "" {
		return fmt.Errorf("%s must be set", EnvHonchoWorkspaceID)
	}
	if c.HonchoBaseURL == "" {
		return errors.New("honcho base url resolved to empty")
	}
	return nil
}
