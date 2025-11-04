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

func checkUnpushedCommits(repo *git.Repository, logger *harukiLogger.Logger) (bool, error) {
	headRef, err := repo.Head()
	if err != nil {
		logger.Errorf("Failed to get HEAD: %v", err)
		return false, err
	}

	remoteRefName := plumbing.NewRemoteReferenceName("origin", headRef.Name().Short())
	remoteRef, err := repo.Reference(remoteRefName, true)
	if err != nil {
		logger.Infof("Remote branch %s not found, assuming there are commits to push", remoteRefName)
		return true, nil
	}

	localHash := headRef.Hash()
	remoteHash := remoteRef.Hash()
	if localHash != remoteHash {
		logger.Infof("Found unpushed commits: local %s vs remote %s", localHash.String(), remoteHash.String())
		return true, nil
	}

	return false, nil
}

func (g *HarukiGitUpdater) commitChanges(w *git.Worktree, dataVersion string, logger *harukiLogger.Logger) (plumbing.Hash, error) {
	commitMsg := fmt.Sprintf("Update data version %s", dataVersion)
	commit, err := w.Commit(commitMsg, &git.CommitOptions{
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
		return plumbing.Hash{}, err
	}
	logger.Infof("Committed changes: %v", commit)
	return commit, nil
}

func (g *HarukiGitUpdater) updateRemoteURL(repo *git.Repository, logger *harukiLogger.Logger) (string, error) {
	remote, err := repo.Remote("origin")
	if err != nil {
		logger.Errorf("Failed to get remote: %v", err)
		return "", err
	}
	remoteConfig := remote.Config()
	origURL := remoteConfig.URLs[0]

	parsed, err := url.Parse(origURL)
	if err != nil {
		logger.Errorf("Failed to parse remote URL: %v", err)
		return "", err
	}
	if g.User != "" && g.Password != "" {
		parsed.User = url.UserPassword(g.User, g.Password)
	}
	newURL := parsed.String()

	remoteConfig.URLs[0] = newURL
	if err := repo.DeleteRemote("origin"); err != nil {
		logger.Errorf("Failed to delete remote: %v", err)
		return "", err
	}
	if _, err := repo.CreateRemote(remoteConfig); err != nil {
		logger.Errorf("Failed to create remote: %v", err)
		return "", err
	}

	return origURL, nil
}

func restoreRemoteURL(repo *git.Repository, origURL string) {
	remote, err := repo.Remote("origin")
	if err != nil {
		return
	}
	remoteConfig := remote.Config()
	remoteConfig.URLs[0] = origURL
	_ = repo.DeleteRemote("origin")
	_, _ = repo.CreateRemote(remoteConfig)
}

func (g *HarukiGitUpdater) setupProxyTransport(logger *harukiLogger.Logger) (func(), error) {
	if g.Proxy == "" {
		return func() {}, nil
	}

	proxyURL, err := url.Parse(g.Proxy)
	if err != nil {
		logger.Errorf("Failed to parse proxy URL: %v", err)
		return nil, err
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

	originalHTTPSTransport, _ := transport.Get("https")
	originalHTTPTransport, _ := transport.Get("http")
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

	cleanup := func() {
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
	}

	logger.Infof("Proxy transport registered successfully: %s", g.Proxy)
	return cleanup, nil
}

func (g *HarukiGitUpdater) PushRemote(repo *git.Repository, dataVersion string) error {
	logger := harukiLogger.NewLogger("HarukiGitUpdater", "INFO", nil)
	w, err := repo.Worktree()
	if err != nil {
		logger.Errorf("Failed to get worktree: %v", err)
		return err
	}

	if err := w.AddWithOptions(&git.AddOptions{All: true}); err != nil {
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
		hasUnpushedCommits, err = checkUnpushedCommits(repo, logger)
		if err != nil {
			return err
		}
	}

	if !hasUncommittedChanges && !hasUnpushedCommits {
		logger.Infof("No changes to commit or push")
		return nil
	}

	if hasUncommittedChanges {
		if _, err := g.commitChanges(w, dataVersion, logger); err != nil {
			return err
		}
	} else {
		logger.Infof("No uncommitted changes, pushing existing commits")
	}

	headRef, err := repo.Head()
	if err != nil {
		logger.Errorf("Failed to get HEAD: %v", err)
		return err
	}
	branchName := headRef.Name().Short()

	origURL, err := g.updateRemoteURL(repo, logger)
	if err != nil {
		return err
	}
	defer restoreRemoteURL(repo, origURL)

	auth := &githttp.BasicAuth{
		Username: g.User,
		Password: g.Password,
	}

	cleanup, err := g.setupProxyTransport(logger)
	if err != nil {
		return err
	}
	defer cleanup()

	pushOpts := &git.PushOptions{
		RemoteName: "origin",
		Auth:       auth,
		RefSpecs:   []config.RefSpec{config.RefSpec(fmt.Sprintf("refs/heads/%s:refs/heads/%s", branchName, branchName))},
		Progress:   os.Stdout,
	}

	if err := repo.Push(pushOpts); err != nil && !strings.Contains(err.Error(), "already up-to-date") {
		logger.Errorf("Failed to push: %v", err)
		return err
	}

	logger.Infof("Pushed changes to remote branch %s", branchName)
	return nil
}
