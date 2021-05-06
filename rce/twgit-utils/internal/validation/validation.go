package validation

import (
	"bytes"
	"strings"
	"text/template"

	log "github.com/sirupsen/logrus"

	"github.com/go-playground/validator/v10"
)

var ValidationDebugErrTemplate = template.Must(
	template.New("validation-debug").Parse(`
Namespace:       {{.Namespace}}
Field:           {{.Field}}
StructNamespace: {{.StructNamespace}}
StructField:     {{.StructField}}
Tag:             {{.Tag}}
ActualTag:       {{.ActualTag}}
Kind:            {{.Kind}}
Type:            {{.Type}}
Value:           {{.Value|printf "%#v"}}
Param:           {{.Param}}
`))

// based on the default Error message, but includes the value in the message
var ValidationHumanTemplate = func() *template.Template {
	s := "Key {{.Namespace}} failed Error:Field " +
		"validation for '{{.Field}}' failed on the {{.Tag}} tag " +
		"for value {{.Value|printf \"%#v\"}}"
	return template.Must(template.New("validation-err").Parse(s))
}()

func NewValidator() *validator.Validate {
	v := validator.New()
	v.SetTagName("v")
	return v
}

// If err is nil, return (nil, nil)
// If err is ValidationErrors, return ([]string, error) with the formatted messages and the original error
// If err is another kind of error, return (nil, error)
func formatValidationErr(err error, t *template.Template) (map[string]string, error) {
	if err == nil {
		return nil, nil
	}
	if errs, ok := err.(validator.ValidationErrors); ok {
		var buf bytes.Buffer
		msgs := make(map[string]string, len(errs))

		for _, e := range errs {
			te := t.Execute(&buf, e)
			if te != nil {
				log.Panicf("[BUG] failed to evaluate validation error template: %#v", te)
			}
			// TODO: this may not work for non-struct errors?
			msgs[e.StructField()] = buf.String()
			buf.Reset()
		}

		return msgs, err
	}

	return nil, err
}

// if t is nil, use the ValidationHumanTemplate
func FormatValidationErrors(err error, t *template.Template) (errs []string) {
	if t == nil {
		t = ValidationHumanTemplate
	}
	msgs, _ := formatValidationErr(err, t)
	if msgs == nil {
		return nil
	}
	for _, msg := range msgs {
		errs = append(errs, msg)
	}
	return errs
}

// if t is nil use the ValidationDebugErrTemplate
func SprintValidationErrors(err error, t *template.Template) string {
	if err == nil {
		return ""
	}
	if t == nil {
		t = ValidationDebugErrTemplate
	}
	m, _ := formatValidationErr(err, t)
	if m == nil {
		return ""
	}
	var msgs []string
	for _, msg := range m {
		msgs = append(msgs, msg)
	}

	return strings.Join(msgs, "\n") + "\n"
}
