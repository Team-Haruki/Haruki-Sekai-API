package git

import (
	"crypto/tls"
	"fmt"
	harukiLogger "haruki-sekai-api/utils/logger"
	"net/http"
	"net/url"
	"os"
	"strings"
	"time"

	"github.com/go-git/go-git/v6"
	"github.com/go-git/go-git/v6/config"
	"github.com/go-git/go-git/v6/plumbing"
	"github.com/go-git/go-git/v6/plumbing/object"
	"github.com/go-git/go-git/v6/plumbing/transport"
	githttp "github.com/go-git/go-git/v6/plumbing/transport/http"
)

type HarukiGitUpdater struct {
	User     string
	Email    string
	Password string
	Proxy    string
}

func NewHarukiGitUpdater(user, email, password, proxy string) *HarukiGitUpdater {
	return &HarukiGitUpdater{
		User:     user,
		Email:    email,
		Password: password,
		Proxy:    proxy,
	}
}

func (g *HarukiGitUpdater) PushRemote(repo *git.Repository, dataVersion string) error {
	logger := harukiLogger.NewLogger("HarukiGitUpdater", "INFO", nil)
	w, err := repo.Worktree()
	if err != nil {
		logger.Errorf("Failed to get worktree: %v", err)
		return err
	}

	err = w.AddWithOptions(&git.AddOptions{All: true})
	if err != nil {
		logger.Errorf("Failed to add changes: %v", err)
		return err
	}

	status, err := w.Status()
	if err != nil {
		logger.Errorf("Failed to get status: %v", err)
		return err
	}

	hasUncommittedChanges := !status.IsClean()
	hasUnpushedCommits := false
	if !hasUncommittedChanges {
		headRef, err := repo.Head()
		if err != nil {
			logger.Errorf("Failed to get HEAD: %v", err)
			return err
		}

		remoteRefName := plumbing.NewRemoteReferenceName("origin", headRef.Name().Short())
		remoteRef, err := repo.Reference(remoteRefName, true)
		if err != nil {
			logger.Infof("Remote branch %s not found, assuming there are commits to push", remoteRefName)
			hasUnpushedCommits = true
		} else {
			localHash := headRef.Hash()
			remoteHash := remoteRef.Hash()
			if localHash != remoteHash {
				hasUnpushedCommits = true
				logger.Infof("Found unpushed commits: local %s vs remote %s", localHash.String(), remoteHash.String())
			}
		}
	}

	if !hasUncommittedChanges && !hasUnpushedCommits {
		logger.Infof("No changes to commit or push")
		return nil
	}

	var commit plumbing.Hash
	if hasUncommittedChanges {
		commitMsg := fmt.Sprintf("Update data version %s", dataVersion)
		commit, err = w.Commit(commitMsg, &git.CommitOptions{
			Author: &object.Signature{
				Name:  "Haruki Sekai Master Update Bot",
				Email: "no-reply@seiunx.com",
				When:  time.Now(),
			},
			Committer: &object.Signature{
				Name:  g.User,
				Email: g.Email,
				When:  time.Now(),
			},
			All: true,
		})
		if err != nil {
			logger.Errorf("Failed to commit: %v", err)
			return err
		}
		logger.Infof("Committed changes: %v", commit)
	} else {
		logger.Infof("No uncommitted changes, pushing existing commits")
	}

	headRef, err := repo.Head()
	if err != nil {
		logger.Errorf("Failed to get HEAD: %v", err)
		return err
	}
	branchName := headRef.Name().Short()

	remote, err := repo.Remote("origin")
	if err != nil {
		logger.Errorf("Failed to get remote: %v", err)
		return err
	}
	remoteConfig := remote.Config()
	origURL := remoteConfig.URLs[0]

	parsed, err := url.Parse(origURL)
	if err != nil {
		logger.Errorf("Failed to parse remote URL: %v", err)
		return err
	}
	if g.User != "" && g.Password != "" {
		parsed.User = url.UserPassword(g.User, g.Password)
	}
	newURL := parsed.String()

	remoteConfig.URLs[0] = newURL
	err = repo.DeleteRemote("origin")
	if err != nil {
		logger.Errorf("Failed to delete remote: %v", err)
		return err
	}
	_, err = repo.CreateRemote(remoteConfig)
	if err != nil {
		logger.Errorf("Failed to create remote: %v", err)
		return err
	}

	auth := &githttp.BasicAuth{
		Username: g.User,
		Password: g.Password,
	}

	var originalHTTPSTransport, originalHTTPTransport transport.Transport
	if g.Proxy != "" {
		proxyURL, err := url.Parse(g.Proxy)
		if err != nil {
			logger.Errorf("Failed to parse proxy URL: %v", err)
			return err
		}

		logger.Infof("Configuring HTTP proxy: %s", g.Proxy)

		customTransport := &http.Transport{
			Proxy: http.ProxyURL(proxyURL),
			TLSClientConfig: &tls.Config{
				InsecureSkipVerify: false,
			},
			TLSHandshakeTimeout:   30 * time.Second,
			ResponseHeaderTimeout: 60 * time.Second,
			IdleConnTimeout:       90 * time.Second,
			ExpectContinueTimeout: 1 * time.Second,
			MaxIdleConns:          10,
			MaxIdleConnsPerHost:   5,
		}

		customClient := &http.Client{
			Transport: customTransport,
			Timeout:   180 * time.Second,
		}
		originalHTTPSTransport, _ = transport.Get("https")
		originalHTTPTransport, _ = transport.Get("http")
		gitTransport := githttp.NewTransport(&githttp.TransportOptions{
			Client: customClient,
		})
		transport.Register("https", gitTransport)
		transport.Register("http", gitTransport)
		originalHTTPProxy := os.Getenv("HTTP_PROXY")
		originalHTTPSProxy := os.Getenv("HTTPS_PROXY")
		originalNoProxy := os.Getenv("NO_PROXY")

		_ = os.Setenv("HTTP_PROXY", g.Proxy)
		_ = os.Setenv("HTTPS_PROXY", g.Proxy)
		_ = os.Setenv("NO_PROXY", "localhost,127.0.0.1,::1")

		defer func() {
			if originalHTTPSTransport != nil {
				transport.Register("https", originalHTTPSTransport)
			}
			if originalHTTPTransport != nil {
				transport.Register("http", originalHTTPTransport)
			}

			if originalHTTPProxy == "" {
				_ = os.Unsetenv("HTTP_PROXY")
			} else {
				_ = os.Setenv("HTTP_PROXY", originalHTTPProxy)
			}
			if originalHTTPSProxy == "" {
				_ = os.Unsetenv("HTTPS_PROXY")
			} else {
				_ = os.Setenv("HTTPS_PROXY", originalHTTPSProxy)
			}
			if originalNoProxy == "" {
				_ = os.Unsetenv("NO_PROXY")
			} else {
				_ = os.Setenv("NO_PROXY", originalNoProxy)
			}
		}()

		logger.Infof("Proxy transport registered successfully: %s", g.Proxy)
	}

	pushOpts := &git.PushOptions{
		RemoteName: "origin",
		Auth:       auth,
		RefSpecs:   []config.RefSpec{config.RefSpec(fmt.Sprintf("refs/heads/%s:refs/heads/%s", branchName, branchName))},
		Progress:   os.Stdout,
	}
	err = repo.Push(pushOpts)
	if err != nil && !strings.Contains(err.Error(), "already up-to-date") {
		logger.Errorf("Failed to push: %v", err)
		remoteConfig.URLs[0] = origURL
		_ = repo.DeleteRemote("origin")
		_, _ = repo.CreateRemote(remoteConfig)
		return err
	}
	logger.Infof("Pushed changes to remote branch %s", branchName)

	remoteConfig.URLs[0] = origURL
	_ = repo.DeleteRemote("origin")
	_, _ = repo.CreateRemote(remoteConfig)
	return nil
}
