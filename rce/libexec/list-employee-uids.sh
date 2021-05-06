#!/bin/bash

ldapsearch -LLLL -x -H ldaps://ldap.local.twitter.com -b cn=users,dc=ods,dc=twitter,dc=corp objectClass=inetOrgPerson uid
