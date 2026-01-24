module LoginIntegrationTest exposing (suite)

{-| Integration tests for the Login page using elm-program-test.
These tests verify the full user flow including form interactions, HTTP requests, and navigation.
-}

import Effect
import Expect
import Http
import Json.Encode as Encode
import Main
import ProgramTest exposing (ProgramTest)
import ProgramTestHelpers as Helpers
import Test exposing (..)


suite : Test
suite =
    describe "Login Page Integration Tests"
        [ describe "Initial Load"
            [ test "shows login form when server is initialized" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.expectHttpRequest "GET" "/api/auth/status"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/auth/status"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.serverStatusJson True "1.0.0")
                        |> Helpers.expectViewHasText "Sign in to your account"
                        |> Helpers.expectViewHasText "Email"
                        |> Helpers.expectViewHasText "Password"
            , test "redirects to setup when server is not initialized" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.expectHttpRequest "GET" "/api/auth/status"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/auth/status"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.serverStatusJson False "1.0.0")
                        |> ProgramTest.expectPageChange "/setup"
            ]
        , describe "Form Interaction"
            [ test "updates email field when typing" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.expectHttpRequest "GET" "/api/auth/status"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/auth/status"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.serverStatusJson True "1.0.0")
                        |> Helpers.fillIn "Email" "user@example.com"
                        |> Helpers.expectViewHasText "user@example.com"
            , test "updates password field when typing" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.expectHttpRequest "GET" "/api/auth/status"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/auth/status"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.serverStatusJson True "1.0.0")
                        |> Helpers.fillIn "Password" "secret123"
                        |> Helpers.expectViewHasText "secret123"
            ]
        , describe "Form Submission"
            [ test "submits login form with correct credentials" <|
                \_ ->
                    let
                        user =
                            { email = "user@example.com"
                            , fullName = "Test User"
                            }

                        authResponse =
                            Helpers.authResponseJson "access-token-123" "refresh-token-456" user
                    in
                    Helpers.createTestProgram
                        |> Helpers.expectHttpRequest "GET" "/api/auth/status"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/auth/status"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.serverStatusJson True "1.0.0")
                        |> Helpers.fillIn "Email" "user@example.com"
                        |> Helpers.fillIn "Password" "password123"
                        |> Helpers.submitForm
                        |> Helpers.ensureHttpRequestWithBody
                            "POST"
                            "/api/auth/login"
                            (Encode.object
                                [ ( "email", Encode.string "user@example.com" )
                                , ( "password", Encode.string "password123" )
                                ]
                            )
                        |> Helpers.simulateHttpOk "POST" "/api/auth/login" authResponse
                        |> ProgramTest.expectPageChange "/dashboard"
            , test "shows error message on login failure" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.expectHttpRequest "GET" "/api/auth/status"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/auth/status"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.serverStatusJson True "1.0.0")
                        |> Helpers.fillIn "Email" "user@example.com"
                        |> Helpers.fillIn "Password" "wrongpassword"
                        |> Helpers.submitForm
                        |> Helpers.ensureHttpRequestWithBody
                            "POST"
                            "/api/auth/login"
                            (Encode.object
                                [ ( "email", Encode.string "user@example.com" )
                                , ( "password", Encode.string "wrongpassword" )
                                ]
                            )
                        |> Helpers.simulateHttpError "POST" "/api/auth/login" (Http.BadStatus 401)
                        |> Helpers.expectViewHasText "Invalid email or password"
                        |> ProgramTest.expectPageChange "/login"
            , test "disables submit button while submitting" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.expectHttpRequest "GET" "/api/auth/status"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/auth/status"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.serverStatusJson True "1.0.0")
                        |> Helpers.fillIn "Email" "user@example.com"
                        |> Helpers.fillIn "Password" "password123"
                        |> Helpers.submitForm
                        |> Helpers.expectViewHasText "Signing in..."
            ]
        , describe "Navigation"
            [ test "stays on login page after failed login" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.expectHttpRequest "GET" "/api/auth/status"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/auth/status"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.serverStatusJson True "1.0.0")
                        |> Helpers.fillIn "Email" "user@example.com"
                        |> Helpers.fillIn "Password" "wrong"
                        |> Helpers.submitForm
                        |> Helpers.expectHttpRequestWithBody
                            "POST"
                            "/api/auth/login"
                            (Encode.object
                                [ ( "email", Encode.string "user@example.com" )
                                , ( "password", Encode.string "wrong" )
                                ]
                            )
                        |> ProgramTest.simulateHttpError
                            (ProgramTest.HttpRequest
                                { method = "POST"
                                , url = "/api/auth/login"
                                , body = ProgramTest.HttpBodyJson
                                    (Encode.object
                                        [ ( "email", Encode.string "user@example.com" )
                                        , ( "password", Encode.string "wrong" )
                                        ]
                                    )
                                , headers = []
                                }
                            )
                            (Http.BadStatus 401)
                        |> ProgramTest.expectPageChange "/login"
            ]
        ]
