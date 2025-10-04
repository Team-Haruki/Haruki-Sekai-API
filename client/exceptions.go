package client

import "fmt"

type SekaiClientException struct {
	msg string
}

func (e *SekaiClientException) Error() string {
	return e.msg
}

type SekaiNoReturnError struct {
	SekaiClientException
}

func NewSekaiNoReturnError() error {
	return &SekaiNoReturnError{SekaiClientException{"Assumed to have return value, but got none."}}
}

type SekaiSignatureError struct {
	SekaiClientException
}

func NewSekaiSignatureError() error {
	return &SekaiSignatureError{SekaiClientException{"Signature error"}}
}

type SekaiAccountError struct {
	SekaiClientException
}

func NewSekaiAccountError() error {
	return &SekaiAccountError{SekaiClientException{"You may not provide any correct accounts."}}
}

type SessionError struct {
	SekaiClientException
}

func NewSessionError() error {
	return &SessionError{SekaiClientException{"Account session error"}}
}

type CookieExpiredError struct {
	SekaiClientException
}

func NewCookieExpiredError() error {
	return &CookieExpiredError{SekaiClientException{"Cookie expired."}}
}

type UpdateRequiredError struct {
	SekaiClientException
}

func NewUpdateRequiredError() error {
	return &UpdateRequiredError{SekaiClientException{"UpdateRequiredError"}}
}

type UpgradeRequiredError struct {
	SekaiClientException
}

func NewUpgradeRequiredError() error {
	return &UpgradeRequiredError{SekaiClientException{"UpgradeRequiredError"}}
}

type UnderMaintenanceError struct {
	SekaiClientException
}

func NewUnderMaintenanceError() error {
	return &UnderMaintenanceError{SekaiClientException{"Game server may under maintenance."}}
}

type UnknownSekaiClientException struct {
	SekaiClientException
	StatusCode int
	Response   string
}

func NewSekaiUnknownClientException(statusCode int, response string) error {
	return &UnknownSekaiClientException{
		SekaiClientException: SekaiClientException{fmt.Sprintf("Unknown error: %d, %s", statusCode, response)},
		StatusCode:           statusCode,
		Response:             response,
	}
}
