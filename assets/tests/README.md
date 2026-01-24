# Integration Tests with elm-program-test

This directory contains integration tests for the Litehouse frontend using `elm-program-test`. These tests follow best practices for testing Elm applications and verify the full user experience including interactions, HTTP requests, and navigation.

## Test Structure

### Test Files

- **`ProgramTestHelpers.elm`** - Reusable helper functions for writing integration tests
- **`LoginIntegrationTest.elm`** - Integration tests for the Login page
- **`DashboardIntegrationTest.elm`** - Integration tests for the Dashboard page
- **`SetupIntegrationTest.elm`** - Integration tests for the Setup page
- **`DashboardTest.elm`** - Unit tests for Dashboard page functions (existing)

## Running Tests

To run all tests:

```bash
cd assets
elm-test
```

To run a specific test file:

```bash
elm-test tests/LoginIntegrationTest.elm
```

## Best Practices Used

### 1. Test Structure

- **Arrange-Act-Assert pattern**: Each test clearly sets up state, performs actions, and verifies results
- **Descriptive test names**: Test names clearly describe what is being tested
- **Grouped by feature**: Tests are organized using `describe` blocks for related functionality

### 2. HTTP Request Testing

- **Stub HTTP requests**: All HTTP requests are stubbed using `ProgramTest.simulateHttpOk` and `ProgramTest.simulateHttpError`
- **Verify request details**: Tests verify that HTTP requests are made with correct method, URL, and body
- **Test error handling**: Both success and error scenarios are tested

### 3. User Interaction Testing

- **Real user actions**: Tests simulate actual user interactions (clicks, form submissions, typing)
- **Form validation**: Tests verify client-side validation works correctly
- **Loading states**: Tests verify that loading states are shown during async operations

### 4. Navigation Testing

- **Route changes**: Tests verify that navigation works correctly using `ProgramTest.expectPageChange`
- **URL-based navigation**: Tests verify that visiting URLs loads the correct page

### 5. View Testing

- **Content verification**: Tests verify that expected content appears in the view
- **Error messages**: Tests verify that error messages are displayed correctly
- **State-dependent UI**: Tests verify that UI changes based on application state

## Helper Functions

The `ProgramTestHelpers` module provides reusable functions:

- `createTestProgram` - Creates a ProgramTest instance with default configuration
- `expectHttpRequest` - Verifies an HTTP request was made
- `expectHttpRequestWithBody` - Verifies an HTTP request with JSON body
- `simulateHttpOk` - Simulates a successful HTTP response
- `simulateHttpError` - Simulates an HTTP error response
- `expectViewHasText` - Verifies text appears in the view
- `expectViewHasNotText` - Verifies text does not appear in the view
- `clickButton` - Simulates clicking a button
- `fillIn` - Simulates filling in an input field
- `submitForm` - Simulates submitting a form

## Example Test

```elm
test "submits login form with correct credentials" <|
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
            |> Helpers.expectHttpRequestWithBody
                "POST"
                "/api/auth/login"
                (Encode.object
                    [ ( "email", Encode.string "user@example.com" )
                    , ( "password", Encode.string "password123" )
                    ]
                )
            |> ProgramTest.simulateHttpOk
                (ProgramTest.HttpRequest
                    { method = "POST"
                    , url = "/api/auth/login"
                    , body = ProgramTest.HttpBodyJson
                        (Encode.object
                            [ ( "email", Encode.string "user@example.com" )
                            , ( "password", Encode.string "password123" )
                            ]
                        )
                    , headers = []
                    }
                )
                authResponse
            |> ProgramTest.expectPageChange "/dashboard"
```

## Testing Philosophy

These integration tests focus on:

1. **User flows**: Testing complete user journeys from start to finish
2. **Integration points**: Verifying that different parts of the application work together
3. **Error scenarios**: Ensuring the application handles errors gracefully
4. **State management**: Verifying that application state is managed correctly

## Notes

- Unit tests (like `DashboardTest.elm`) test individual functions in isolation
- Integration tests (like `*IntegrationTest.elm`) test the full application flow
- Both types of tests are valuable and complement each other
- Integration tests are slower but provide more confidence that the application works end-to-end
