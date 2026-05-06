{{/*
Expand the name of the chart.
*/}}
{{- define "haruki.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/*
Create a fully qualified app name (release-name + chart-name).
*/}}
{{- define "haruki.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- if contains $name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{- define "haruki.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/*
Common labels.
*/}}
{{- define "haruki.labels" -}}
helm.sh/chart: {{ include "haruki.chart" . }}
{{ include "haruki.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{- define "haruki.selectorLabels" -}}
app.kubernetes.io/name: {{ include "haruki.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "haruki.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "haruki.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{/*
Image reference.
*/}}
{{- define "haruki.image" -}}
{{- $tag := default .Chart.AppVersion .Values.image.tag -}}
{{- printf "%s:%s" .Values.image.repository $tag -}}
{{- end -}}

{{- define "haruki.masterClaimName" -}}
{{- default (printf "%s-master" (include "haruki.fullname" .)) .Values.persistence.master.existingClaim -}}
{{- end -}}

{{- define "haruki.accountsClaimName" -}}
{{- default (printf "%s-accounts" (include "haruki.fullname" .)) .Values.persistence.accounts.existingClaim -}}
{{- end -}}

{{/*
Render the env block shared by both Deployments. Includes:
  - Plain env from .Values.env
  - HARUKI_*_FILE entries derived from .Values.secrets.fileEnv
  - CONFIG_PATH if .Values.configFile.enabled
*/}}
{{- define "haruki.envBlock" -}}
{{- range $k, $v := .Values.env }}
- name: {{ $k }}
  value: {{ $v | quote }}
{{- end }}
{{- if and .Values.secrets.existing .Values.secrets.fileEnv }}
{{- range $key, $envName := .Values.secrets.fileEnv }}
- name: {{ $envName }}
  value: {{ printf "%s/%s" $.Values.secrets.mountPath $key | quote }}
{{- end }}
{{- end }}
{{- if .Values.configFile.enabled }}
- name: CONFIG_PATH
  value: {{ .Values.configFile.mountPath | quote }}
{{- end }}
{{- end -}}

{{/*
Render the updater env block. It is the same as haruki.envBlock, except the
API-side updater toggle is omitted because the updater Deployment forces it on.
*/}}
{{- define "haruki.updaterEnvBlock" -}}
{{- range $k, $v := .Values.env }}
{{- if ne $k "HARUKI_BACKEND__RUN_UPDATERS_INPROC" }}
- name: {{ $k }}
  value: {{ $v | quote }}
{{- end }}
{{- end }}
{{- if and .Values.secrets.existing .Values.secrets.fileEnv }}
{{- range $key, $envName := .Values.secrets.fileEnv }}
- name: {{ $envName }}
  value: {{ printf "%s/%s" $.Values.secrets.mountPath $key | quote }}
{{- end }}
{{- end }}
{{- if .Values.configFile.enabled }}
- name: CONFIG_PATH
  value: {{ .Values.configFile.mountPath | quote }}
{{- end }}
{{- end -}}

{{/*
Secret/config volumes shared by both Deployments.
*/}}
{{- define "haruki.commonVolumes" -}}
{{- if .Values.secrets.existing }}
- name: secrets
  secret:
    secretName: {{ .Values.secrets.existing }}
{{- end }}
{{- if .Values.configFile.enabled }}
- name: configfile
  secret:
    secretName: {{ .Values.configFile.existingSecret }}
    items:
      - key: {{ .Values.configFile.key }}
        path: {{ base .Values.configFile.mountPath }}
{{- end }}
{{- end -}}

{{- define "haruki.commonVolumeMounts" -}}
{{- if .Values.secrets.existing }}
- name: secrets
  mountPath: {{ .Values.secrets.mountPath }}
  readOnly: true
{{- end }}
{{- if .Values.configFile.enabled }}
- name: configfile
  mountPath: {{ .Values.configFile.mountPath }}
  subPath: {{ base .Values.configFile.mountPath }}
  readOnly: true
{{- end }}
{{- end -}}

{{- define "haruki.apiVolumes" -}}
{{- include "haruki.commonVolumes" . }}
{{- if .Values.persistence.accounts.enabled }}
- name: accounts
  persistentVolumeClaim:
    claimName: {{ include "haruki.accountsClaimName" . }}
{{- end }}
{{- end -}}

{{- define "haruki.apiVolumeMounts" -}}
{{- include "haruki.commonVolumeMounts" . }}
{{- if .Values.persistence.accounts.enabled }}
- name: accounts
  mountPath: {{ .Values.persistence.accounts.mountPath | quote }}
{{- end }}
{{- end -}}

{{- define "haruki.updaterVolumes" -}}
{{- include "haruki.commonVolumes" . }}
{{- if .Values.persistence.accounts.enabled }}
- name: accounts
  persistentVolumeClaim:
    claimName: {{ include "haruki.accountsClaimName" . }}
{{- end }}
{{- if .Values.persistence.master.enabled }}
- name: master
  persistentVolumeClaim:
    claimName: {{ include "haruki.masterClaimName" . }}
{{- end }}
{{- end -}}

{{- define "haruki.updaterVolumeMounts" -}}
{{- include "haruki.commonVolumeMounts" . }}
{{- if .Values.persistence.accounts.enabled }}
- name: accounts
  mountPath: {{ .Values.persistence.accounts.mountPath | quote }}
{{- end }}
{{- if .Values.persistence.master.enabled }}
- name: master
  mountPath: {{ .Values.persistence.master.mountPath | quote }}
{{- end }}
{{- end -}}
