{{- define "relay.apiKey" -}}
{{- $existing := (lookup "v1" "Secret" .Release.Namespace "relay-api-key") -}}
{{- if $existing -}}
{{- index $existing.data "api-key" | b64dec -}}
{{- else -}}
{{- randAlphaNum 32 -}}
{{- end -}}
{{- end -}}
