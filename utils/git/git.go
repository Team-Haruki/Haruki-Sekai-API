package git

import (
	"fmt"
	harukiLogger "haruki-sekai-api/utils/logger"
	"net/url"
	"os"
	"strings"
	"time"

	"github.com/go-git/go-git/v5"
	"github.com/go-git/go-git/v5/config"
	"github.com/go-git/go-git/v5/plumbing/object"
	"github.com/go-git/go-git/v5/plumbing/transport/http"
)

type GitUpdater struct {
	User     string
	Email    string
	Password string
	Proxy    string
}

func (g *GitUpdater) PushRemote(repo *git.Repository, dataVersion string) error {
	logger := harukiLogger.NewLogger("GitUpdater", "INFO", nil)
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
	if status.IsClean() {
		logger.Infof("No changes to commit")
		return nil
	}

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
		return err
	}
	logger.Infof("Committed changes: %v", commit)

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

	pushOpts := &git.PushOptions{
		RemoteName: "origin",
		Auth: &http.BasicAuth{
			Username: g.User,
			Password: g.Password,
		},
		RefSpecs: []config.RefSpec{config.RefSpec(fmt.Sprintf("refs/heads/%s:refs/heads/%s", branchName, branchName))},
		Progress: os.Stdout,
	}
	if g.Proxy != "" {
		os.Setenv("HTTP_PROXY", g.Proxy)
		os.Setenv("HTTPS_PROXY", g.Proxy)
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
